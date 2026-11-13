//! Persistent deployment transaction state and cold data-volume snapshots.
//!
//! A full deployment either commits or restores the previous version. The journal is
//! written before every destructive step so a desktop crash can be diagnosed and a
//! previous-version recovery can be started without guessing what had been replaced.

use crate::core::{atomic_file, path_guard, process};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const TRANSACTIONS_DIR: &str = ".deploy-desktop/transactions";
pub const DEPLOY_LOCK: &str = ".deploy-desktop/deploy.lock";
const DEPLOYED_RECORD_PATH: &str = ".deploy-desktop/deployed.json";
pub const PHASE_PREPARING: &str = "preparing";
pub const PHASE_SNAPSHOTTING_FILES: &str = "snapshotting_files";
pub const PHASE_SNAPSHOTTING_DATA: &str = "snapshotting_data";
pub const PHASE_APPLYING: &str = "applying";
pub const PHASE_STARTING: &str = "starting";
pub const PHASE_COMMITTED: &str = "committed";
pub const PHASE_RECOVERING: &str = "recovering";
pub const PHASE_RECOVERED: &str = "recovered";
pub const PHASE_RECOVERY_FAILED: &str = "recovery_failed";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentTransaction {
    pub schema_version: u32,
    pub id: String,
    pub product: String,
    pub version: String,
    #[serde(default)]
    pub previous_version: String,
    pub install_dir: String,
    pub deploy_dir: String,
    pub phase: String,
    pub state: String,
    pub started_at: String,
    pub updated_at: String,
    pub snapshot_dir: String,
    pub had_deployment: bool,
    pub project_name: String,
    #[serde(default)]
    pub previous_compose: String,
    #[serde(default)]
    pub new_compose: String,
    #[serde(default)]
    pub files: Vec<FileSnapshot>,
    #[serde(default)]
    pub volumes: Vec<VolumeSnapshot>,
    #[serde(default)]
    pub old_images: Vec<ImageSnapshot>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub previous_services: Vec<String>,
    #[serde(default)]
    pub frontend_service: String,
    #[serde(default)]
    pub frontend_port: Option<u16>,
    #[serde(default)]
    pub startup_attempted: bool,
    #[serde(default)]
    pub deployment_started: bool,
    #[serde(default)]
    pub applied_paths: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub recovery_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSnapshot {
    pub path: String,
    pub existed: bool,
    pub kind: String,
    pub bytes: u64,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeSnapshot {
    pub logical_name: String,
    pub actual_name: String,
    pub image: String,
    pub existed: bool,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSnapshot {
    pub reference: String,
    pub image_id: String,
    #[serde(default)]
    pub platform: String,
}

impl DeploymentTransaction {
    pub fn new(
        install_dir: &Path,
        deploy_dir: &Path,
        product: &str,
        version: &str,
        previous_version: &str,
        had_deployment: bool,
        project_name: &str,
        previous_compose: &str,
        new_compose: &str,
    ) -> Result<Self, String> {
        let now = now_text();
        let id = format!(
            "deploy-{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_millis())
                .unwrap_or(0),
            crate::core::backup::unique_suffix()
        );
        let snapshot_dir = transaction_dir(install_dir, &id).join("snapshot");
        fs::create_dir_all(snapshot_dir.join("files"))
            .map_err(|error| format!("创建部署事务目录失败：{error}"))?;
        fs::create_dir_all(snapshot_dir.join("volumes"))
            .map_err(|error| format!("创建部署事务目录失败：{error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                transaction_dir(install_dir, &id),
                fs::Permissions::from_mode(0o700),
            )
            .map_err(|error| format!("设置部署事务目录权限失败：{error}"))?;
        }
        Ok(Self {
            schema_version: 1,
            id,
            product: product.to_string(),
            version: version.to_string(),
            previous_version: previous_version.to_string(),
            install_dir: install_dir.to_string_lossy().to_string(),
            deploy_dir: deploy_dir.to_string_lossy().to_string(),
            phase: PHASE_PREPARING.to_string(),
            state: "running".to_string(),
            started_at: now.clone(),
            updated_at: now,
            snapshot_dir: snapshot_dir.to_string_lossy().to_string(),
            had_deployment,
            project_name: project_name.to_string(),
            previous_compose: previous_compose.to_string(),
            new_compose: new_compose.to_string(),
            files: Vec::new(),
            volumes: Vec::new(),
            old_images: Vec::new(),
            services: Vec::new(),
            previous_services: Vec::new(),
            frontend_service: String::new(),
            frontend_port: None,
            startup_attempted: false,
            deployment_started: false,
            applied_paths: Vec::new(),
            error: None,
            recovery_error: None,
        })
    }

    pub fn snapshot_path(&self) -> PathBuf {
        PathBuf::from(&self.snapshot_dir)
    }

    pub fn set_phase(&mut self, install_dir: &Path, phase: &str) -> Result<(), String> {
        self.phase = phase.to_string();
        self.updated_at = now_text();
        write_transaction(install_dir, self)
    }

    pub fn fail(&mut self, install_dir: &Path, error: &str) -> Result<(), String> {
        self.state = "failed".to_string();
        self.error = Some(error.to_string());
        self.updated_at = now_text();
        write_transaction(install_dir, self)
    }

    pub fn recover(&mut self, install_dir: &Path) -> Result<(), String> {
        self.phase = PHASE_RECOVERING.to_string();
        self.state = "recovering".to_string();
        self.updated_at = now_text();
        write_transaction(install_dir, self)
    }

    pub fn recovered(&mut self, install_dir: &Path) -> Result<(), String> {
        self.phase = PHASE_RECOVERED.to_string();
        self.state = "recovered".to_string();
        self.updated_at = now_text();
        write_transaction(install_dir, self)
    }

    pub fn recovery_failed(&mut self, install_dir: &Path, error: &str) -> Result<(), String> {
        self.phase = PHASE_RECOVERY_FAILED.to_string();
        self.state = "recovery_failed".to_string();
        self.recovery_error = Some(error.to_string());
        self.updated_at = now_text();
        write_transaction(install_dir, self)
    }
}

pub struct DeploymentLock {
    _file: File,
}

impl Drop for DeploymentLock {
    fn drop(&mut self) {
        let _ = self._file.unlock();
    }
}

impl DeploymentLock {
    pub fn acquire(install_dir: &Path) -> Result<Self, String> {
        Self::acquire_path(install_dir.join(DEPLOY_LOCK))
    }

    pub fn acquire_project(project_name: &str) -> Result<Self, String> {
        if project_name.trim().is_empty() {
            return Err("Compose 项目名称为空，无法建立部署锁".to_string());
        }
        Self::acquire_path(project_lock_path(project_name))
    }

    fn acquire_path(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if fs::symlink_metadata(parent)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err("部署锁目录不能使用符号链接".to_string());
            }
            fs::create_dir_all(parent).map_err(|error| format!("创建部署锁目录失败：{error}"))?;
        }
        if fs::symlink_metadata(&path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err("部署锁不能使用符号链接".to_string());
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| format!("无法创建部署锁：{error}"))?;
        file.try_lock().map_err(|_| {
            "同一部署目录已有任务正在执行，请等待结束或先检查未完成事务".to_string()
        })?;
        file.set_len(0)
            .and_then(|()| writeln!(file, "{}", std::process::id()))
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("无法写入部署锁：{error}"))?;
        Ok(Self { _file: file })
    }
}

pub fn project_lock_path(project_name: &str) -> PathBuf {
    let digest = super::bundle::sha256_hex(project_name.trim().as_bytes());
    std::env::temp_dir()
        .join("deploy-desktop-project-locks")
        .join(format!("{digest}.lock"))
}

pub fn transaction_dir(install_dir: &Path, id: &str) -> PathBuf {
    install_dir.join(TRANSACTIONS_DIR).join(id)
}

pub fn journal_path(install_dir: &Path, id: &str) -> PathBuf {
    transaction_dir(install_dir, id).join("journal.json")
}

pub fn write_transaction(
    install_dir: &Path,
    transaction: &DeploymentTransaction,
) -> Result<(), String> {
    let path = journal_path(install_dir, &transaction.id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建部署事务目录失败：{error}"))?;
    }
    let text = serde_json::to_string_pretty(transaction)
        .map_err(|error| format!("写入部署事务失败：{error}"))?;
    atomic_file::write_text_atomic(&path, &text, atomic_file::FilePermission::OwnerOnly)
        .map_err(|error| format!("写入部署事务失败：{error}"))
}

pub fn latest_transaction(install_dir: &Path) -> Result<Option<DeploymentTransaction>, String> {
    let transactions = transactions(install_dir)?;
    Ok(transactions
        .iter()
        .find(|transaction| !matches!(transaction.state.as_str(), "committed" | "recovered"))
        .cloned()
        .or_else(|| transactions.into_iter().next()))
}

fn transactions(install_dir: &Path) -> Result<Vec<DeploymentTransaction>, String> {
    let root = install_dir.join(TRANSACTIONS_DIR);
    let directory = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("读取部署事务目录失败：{error}")),
    };
    let mut entries: Vec<_> = directory
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取部署事务目录失败：{error}"))?
        .into_iter()
        .filter(|entry| {
            entry.path().is_dir()
                && entry.file_name().to_str().is_some_and(|name| {
                    name.starts_with("deploy-")
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
        })
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    let mut transactions = Vec::with_capacity(entries.len());
    for entry in entries.into_iter().rev() {
        let path = entry.path().join("journal.json");
        if !path.exists() {
            if entry.path().join("status.env").exists() {
                return Err(format!(
                    "部署事务目录 {} 缺少 journal 但存在运行状态，请检查现场后再部署",
                    entry.path().display()
                ));
            }
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|error| {
            format!(
                "部署事务目录 {} 的记录无法读取：{error}。请检查现场后再部署",
                entry.path().display()
            )
        })?;
        let transaction = serde_json::from_str(&text).map_err(|_| {
            format!(
                "部署事务目录 {} 的记录格式无效。请检查现场后再部署",
                entry.path().display()
            )
        })?;
        transactions.push(transaction);
    }
    Ok(transactions)
}

pub fn incomplete_transaction(install_dir: &Path) -> Result<Option<DeploymentTransaction>, String> {
    Ok(transactions(install_dir)?
        .into_iter()
        .find(|transaction| !matches!(transaction.state.as_str(), "committed" | "recovered")))
}

pub fn ensure_no_incomplete_transaction(install_dir: &Path) -> Result<(), String> {
    match incomplete_transaction(install_dir)? {
        Some(transaction) if transaction.state == "recovery_failed" => Err(format!(
            "检测到恢复失败的部署事务 {}（阶段：{}）。现场保留在 {}，请按快照目录人工处理后再部署",
            transaction.id, transaction.phase, transaction.snapshot_dir
        )),
        Some(transaction) if transaction.state == "recovering" => Err(format!(
            "检测到恢复被中断的部署事务 {}（阶段：{}）。现场保留在 {}，请检查恢复日志并人工处理后再部署",
            transaction.id, transaction.phase, transaction.snapshot_dir
        )),
        Some(transaction) => Err(format!(
            "检测到未完成部署事务 {}（阶段：{}）。请先恢复上一版本后再部署",
            transaction.id, transaction.phase
        )),
        None => Ok(()),
    }
}

pub fn relative_install_path(install_dir: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(install_dir)
        .map_err(|_| "部署事务路径超出部署目录".to_string())?;
    if relative.as_os_str().is_empty() || path_guard::has_parent_dir(relative) {
        return Err("部署事务路径无效".to_string());
    }
    let relative = relative.to_string_lossy().replace('\\', "/");
    validate_managed_relative(&relative)?;
    Ok(relative)
}

pub fn snapshot_managed_files(
    install_dir: &Path,
    transaction: &mut DeploymentTransaction,
    relative_paths: &[String],
) -> Result<(), String> {
    let root = transaction.snapshot_path().join("files");
    let mut paths = relative_paths.to_vec();
    paths.sort();
    paths.dedup();
    for relative in paths {
        reject_relative(&relative)?;
        let source = install_dir.join(&relative);
        let destination = root.join(&relative);
        if !source.exists() {
            transaction.files.push(FileSnapshot {
                path: relative,
                existed: false,
                kind: "missing".to_string(),
                bytes: 0,
                sha256: String::new(),
            });
            write_transaction(install_dir, transaction)?;
            continue;
        }
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("读取待备份路径 {relative} 失败：{error}"))?;
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(format!(
                "部署路径 {relative} 含符号链接或特殊文件，无法安全备份"
            ));
        }
        copy_tree(&source, &destination)?;
        let bytes = tree_bytes(&source)?;
        let sha256 = hash_path(&source)?;
        if tree_bytes(&destination)? != bytes || hash_path(&destination)? != sha256 {
            return Err(format!("文件快照 {relative} 校验失败，未开始写入新版本"));
        }
        transaction.files.push(FileSnapshot {
            path: relative,
            existed: true,
            kind: if metadata.is_dir() { "dir" } else { "file" }.to_string(),
            bytes,
            sha256,
        });
        write_transaction(install_dir, transaction)?;
    }
    Ok(())
}

pub fn restore_managed_files(
    install_dir: &Path,
    transaction: &DeploymentTransaction,
) -> Result<(), String> {
    let root = transaction.snapshot_path().join("files");
    for entry in &transaction.files {
        reject_relative(&entry.path)?;
        let target = install_dir.join(&entry.path);
        remove_path(&target)?;
        if !entry.existed {
            continue;
        }
        let source = root.join(&entry.path);
        if !source.exists() {
            return Err(format!("部署事务缺少文件快照：{}", entry.path));
        }
        copy_tree(&source, &target)?;
    }
    Ok(())
}

pub fn verify_recovery_snapshot(transaction: &DeploymentTransaction) -> Result<(), String> {
    let file_root = transaction.snapshot_path().join("files");
    for entry in &transaction.files {
        reject_relative(&entry.path)?;
        if !entry.existed {
            continue;
        }
        let source = file_root.join(&entry.path);
        let metadata = fs::symlink_metadata(&source)
            .map_err(|_| format!("部署事务缺少文件快照：{}", entry.path))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "文件快照 {} 包含符号链接，无法安全恢复",
                entry.path
            ));
        }
        let kind = if metadata.is_dir() {
            "dir"
        } else if metadata.is_file() {
            "file"
        } else {
            return Err(format!("文件快照 {} 类型无效，无法安全恢复", entry.path));
        };
        if kind != entry.kind {
            return Err(format!("文件快照 {} 类型与事务记录不一致", entry.path));
        }
        let bytes = tree_bytes(&source)?;
        if bytes != entry.bytes {
            return Err(format!("文件快照 {} 大小校验失败", entry.path));
        }
        if hash_path(&source)? != entry.sha256 {
            return Err(format!("文件快照 {} 完整性校验失败", entry.path));
        }
    }
    for volume in &transaction.volumes {
        if !volume.existed {
            continue;
        }
        let backup = Path::new(&volume.file);
        let metadata = fs::metadata(backup)
            .map_err(|_| format!("数据卷 {} 的备份文件不可读", volume.logical_name))?;
        if metadata.len() != volume.bytes {
            return Err(format!("数据卷 {} 的备份大小校验失败", volume.logical_name));
        }
        if crate::commands::bundle::sha256_path(backup)? != volume.sha256 {
            return Err(format!(
                "数据卷 {} 的备份完整性校验失败",
                volume.logical_name
            ));
        }
        verify_gzip_tar(backup)
            .map_err(|error| format!("数据卷 {} 的备份无法恢复：{error}", volume.logical_name))?;
    }
    Ok(())
}

pub fn remove_managed_files(targets: &[PathBuf]) -> Result<(), String> {
    let mut unique: Vec<&PathBuf> = targets.iter().collect();
    unique.sort();
    unique.dedup();
    for target in unique {
        remove_path(target)?;
    }
    Ok(())
}

pub fn backup_volume_local(
    image: &str,
    actual_name: &str,
    destination: &Path,
) -> Result<(String, u64), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建数据卷备份目录失败：{error}"))?;
    }
    let mut command = crate::commands::docker_setup::docker_command();
    command
        .args(["run", "--rm", "--network", "none", "-v"])
        .arg(format!("{actual_name}:/data:ro"))
        .arg("--entrypoint")
        .arg("/bin/sh")
        .arg(image)
        .args(["-c", "tar -czf - -C /data ."]);
    let stderr = run_to_file_with_timeout(
        command,
        destination,
        Duration::from_secs(6 * 60 * 60),
    )
    .map_err(|error| format!("数据卷 {actual_name} 备份失败：{error}"))?;
    let _ = stderr;
    verify_gzip_tar(destination)?;
    let bytes = fs::metadata(destination)
        .map_err(|error| format!("读取数据卷备份大小失败：{error}"))?
        .len();
    Ok((crate::commands::bundle::sha256_path(destination)?, bytes))
}

fn run_to_file_with_timeout(
    mut command: std::process::Command,
    destination: &Path,
    timeout: Duration,
) -> Result<String, String> {
    let output =
        File::create(destination).map_err(|error| format!("创建备份文件失败：{error}"))?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(output))
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动备份进程失败：{error}"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取备份进程错误输出".to_string())?;
    let reader = std::thread::spawn(move || {
        let mut captured = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let read = match stderr.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => break,
            };
            if captured.len() < 64 * 1024 {
                let remaining = 64 * 1024 - captured.len();
                captured.extend_from_slice(&buffer[..read.min(remaining)]);
            }
        }
        String::from_utf8_lossy(&captured).trim().to_string()
    });
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("等待备份进程失败：{error}"))?
        {
            let stderr = reader.join().unwrap_or_default();
            if !status.success() {
                let _ = fs::remove_file(destination);
                return Err(if stderr.is_empty() {
                    format!("备份进程退出码 {}", status.code().unwrap_or(1))
                } else {
                    stderr
                });
            }
            return Ok(stderr);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = fs::remove_file(destination);
            return Err("备份超时，已停止备份进程".to_string());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn restore_volume_local(image: &str, actual_name: &str, backup: &Path) -> Result<(), String> {
    verify_gzip_tar(backup)?;
    let mut command = crate::commands::docker_setup::docker_command();
    command
        .args(["run", "--rm", "--network", "none", "-i", "-v"])
        .arg(format!("{actual_name}:/data"))
        .arg("--entrypoint")
        .arg("/bin/sh")
        .arg(image)
        .args([
            "-c",
            "find /data -mindepth 1 -maxdepth 1 -exec rm -rf -- {} + 2>/dev/null || true; [ -z \"$(find /data -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)\" ] && tar -xzf - -C /data",
        ]);
    let output = process::run(
        command,
        process::Input::File {
            path: backup.to_path_buf(),
            progress: None,
        },
        process::Options {
            timeout: Duration::from_secs(6 * 60 * 60),
            ..process::Options::default()
        },
    )?;
    if !output.status.success() {
        return Err(format!(
            "恢复数据卷 {actual_name} 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

pub fn verify_gzip_tar(path: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|error| format!("数据卷备份不可读：{error}"))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| format!("数据卷备份不是有效压缩包：{error}"))?;
    let mut count = 0usize;
    for entry in entries {
        let entry = entry.map_err(|error| format!("数据卷备份完整性校验失败：{error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("数据卷备份包含无效路径：{error}"))?;
        if path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err("数据卷备份包含越界路径，已停止恢复".to_string());
        }
        count += 1;
    }
    if count == 0 {
        return Err("数据卷备份为空，无法确认完整性".to_string());
    }
    let mut decoder = archive.into_inner();
    std::io::copy(&mut decoder, &mut std::io::sink())
        .map_err(|error| format!("数据卷备份压缩流完整性校验失败：{error}"))?;
    Ok(())
}

pub fn check_backup_tools_local(image: &str) -> Result<(), String> {
    let mut command = crate::commands::docker_setup::docker_command();
    command
        .args([
            "run",
            "--rm",
            "--network",
            "none",
            "--entrypoint",
            "/bin/sh",
            image,
            "-c",
            "command -v tar >/dev/null && command -v gzip >/dev/null",
        ])
        .stdin(Stdio::null());
    let output = command
        .output()
        .map_err(|error| format!("检查数据卷备份镜像失败：{error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "数据卷备份镜像 {image} 缺少 tar 或 gzip，无法在停服前建立可靠备份"
        ))
    }
}

fn reject_relative(path: &str) -> Result<(), String> {
    validate_managed_relative(path)
}

pub fn validate_managed_relative(path: &str) -> Result<(), String> {
    path_guard::check_relative(path)
        .map_err(|_| "部署事务包含无效路径".to_string())?;
    let path = path.replace('\\', "/");
    if super::bundle::is_internal_deploy_path(&path) && path != DEPLOYED_RECORD_PATH {
        return Err("部署事务路径指向内部状态目录".to_string());
    }
    Ok(())
}

pub fn validate_artifact_relative(path: &str) -> Result<(), String> {
    validate_managed_relative(path)?;
    if super::bundle::is_internal_deploy_path(path) {
        return Err("部署事务路径指向内部状态目录".to_string());
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(source).map_err(|error| format!("读取备份来源失败：{error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("部署目录包含符号链接，无法安全备份".to_string());
    }
    if metadata.is_dir() {
        fs::create_dir_all(target).map_err(|error| format!("创建文件快照目录失败：{error}"))?;
        for entry in fs::read_dir(source).map_err(|error| format!("读取备份目录失败：{error}"))?
        {
            let entry = entry.map_err(|error| format!("读取备份文件失败：{error}"))?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建文件快照目录失败：{error}"))?;
        }
        retry_file_operation(|| fs::copy(source, target))
            .map_err(|error| format!("复制部署文件到快照失败：{error}"))?;
    } else {
        return Err("部署目录包含不支持的特殊文件".to_string());
    }
    Ok(())
}

pub fn remove_deployment_path(path: &Path) -> Result<(), String> {
    retry_file_operation(|| remove_path_once(path))
        .map_err(|error| format!("清理部署路径 {} 失败：{error}", path.display()))
}

pub fn retry_file_operation<T>(
    mut operation: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    for attempt in 0..40 {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if is_file_lock_error(&error) && attempt < 39 => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn remove_path_once(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)
        }
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn is_file_lock_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

fn remove_path(path: &Path) -> Result<(), String> {
    remove_deployment_path(path)
}

fn tree_bytes(path: &Path) -> Result<u64, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("读取备份大小失败：{error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("部署目录包含符号链接，无法安全备份".to_string());
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path).map_err(|error| format!("读取备份目录大小失败：{error}"))?
    {
        total = total.saturating_add(tree_bytes(
            &entry
                .map_err(|error| format!("读取备份目录大小失败：{error}"))?
                .path(),
        )?);
    }
    Ok(total)
}

fn hash_path(path: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hash_path_into(path, path, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn hash_path_into(path: &Path, root: &Path, hasher: &mut Sha256) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("读取备份摘要失败：{error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("文件快照包含符号链接，无法安全恢复".to_string());
    }
    let relative = path
        .strip_prefix(root)
        .unwrap_or(Path::new("."))
        .to_string_lossy()
        .replace('\\', "/");
    hasher.update((relative.len() as u64).to_le_bytes());
    hasher.update(relative.as_bytes());
    if metadata.is_dir() {
        hasher.update(b"dir");
        let mut entries = fs::read_dir(path)
            .map_err(|error| format!("读取备份目录摘要失败：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("读取备份摘要失败：{error}"))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            hash_path_into(&entry.path(), root, hasher)?;
        }
    } else if metadata.is_file() {
        hasher.update(b"file");
        hasher.update(metadata.len().to_le_bytes());
        let mut file =
            File::open(path).map_err(|error| format!("读取备份文件摘要失败：{error}"))?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("读取备份文件摘要失败：{error}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    Ok(())
}

fn now_text() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_lock_is_exclusive_and_released_by_closing_file() {
        let install = temp_dir("exclusive");
        let lock = DeploymentLock::acquire(&install).unwrap();
        assert!(DeploymentLock::acquire(&install).is_err());
        drop(lock);
        assert!(DeploymentLock::acquire(&install).is_ok());
    }

    #[test]
    fn project_lock_is_shared_across_install_directories() {
        let project = format!(
            "project-lock-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let lock = DeploymentLock::acquire_project(&project).unwrap();
        assert!(DeploymentLock::acquire_project(&project).is_err());
        drop(lock);
        assert!(DeploymentLock::acquire_project(&project).is_ok());
    }

    #[test]
    fn successful_snapshot_cleanup_preserves_the_journal() {
        let install = temp_dir("committed-journal");
        let mut transaction = new_transaction(&install);
        transaction.state = "committed".to_string();
        transaction.phase = PHASE_COMMITTED.to_string();
        write_transaction(&install, &transaction).unwrap();
        fs::remove_dir_all(transaction.snapshot_path()).unwrap();
        assert_eq!(
            latest_transaction(&install).unwrap().unwrap().state,
            "committed"
        );
    }

    #[test]
    fn directory_hash_is_independent_of_creation_order() {
        let first = temp_dir("hash-first");
        let second = temp_dir("hash-second");
        for name in ["z", "a", "m"] {
            fs::write(first.join(name), name).unwrap();
        }
        for name in ["m", "a", "z"] {
            fs::write(second.join(name), name).unwrap();
        }
        assert_eq!(hash_path(&first).unwrap(), hash_path(&second).unwrap());
    }

    #[test]
    fn transaction_paths_reject_internal_state_and_dot_entries() {
        assert!(validate_managed_relative(".").is_err());
        assert!(validate_managed_relative(".deploy-desktop").is_err());
        assert!(validate_managed_relative(".deploy-desktop/transactions/deploy-1").is_err());
        assert!(validate_managed_relative(".DEPLOY-DESKTOP/deployed.json").is_err());
        assert!(validate_managed_relative(".deploy-desktop./transactions/deploy-1").is_err());
        assert!(validate_managed_relative(".deploy-desktop /transactions/deploy-1").is_err());
        assert!(validate_managed_relative(".deploy-desktop:stream").is_err());
        assert!(validate_managed_relative(".deploy-desktop/deployed.json").is_ok());
        assert!(validate_managed_relative("deploy/docker-compose.yml").is_ok());
        assert!(validate_artifact_relative(".deploy-desktop/deployed.json").is_err());
        assert!(validate_artifact_relative("deploy/docker-compose.yml").is_ok());
    }

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "deployment-tx-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn new_transaction(install: &Path) -> DeploymentTransaction {
        DeploymentTransaction::new(
            install,
            &install.join("deploy"),
            "demo",
            "2.0.0",
            "1.0.0",
            true,
            "demo",
            "docker-compose.yml",
            "docker-compose.yml",
        )
        .unwrap()
    }

    #[test]
    fn unfinished_and_recovery_failed_transactions_block_new_deployments() {
        let install = temp_dir("journal");
        let mut transaction = new_transaction(&install);
        write_transaction(&install, &transaction).unwrap();
        assert!(incomplete_transaction(&install).unwrap().is_some());

        transaction.recovered(&install).unwrap();
        assert!(incomplete_transaction(&install).unwrap().is_none());
        assert!(ensure_no_incomplete_transaction(&install).is_ok());

        let mut failed = new_transaction(&install);
        failed.recovery_failed(&install, "数据卷恢复失败").unwrap();
        assert!(incomplete_transaction(&install).unwrap().is_some());
        let message = ensure_no_incomplete_transaction(&install).unwrap_err();
        assert!(message.contains("人工处理"));
    }

    #[test]
    fn orphan_transaction_directory_without_journal_does_not_block_deployment() {
        let install = temp_dir("orphan");
        let orphan = transaction_dir(&install, "deploy-orphan");
        fs::create_dir_all(orphan.join("incoming")).unwrap();
        assert!(ensure_no_incomplete_transaction(&install).is_ok());
        fs::write(orphan.join("status.env"), "state=running\n").unwrap();
        assert!(ensure_no_incomplete_transaction(&install).is_err());
    }

    #[test]
    fn invalid_transaction_journal_blocks_new_deployments() {
        let install = temp_dir("invalid-journal");
        let transaction = new_transaction(&install);
        write_transaction(&install, &transaction).unwrap();
        fs::write(journal_path(&install, &transaction.id), "{").unwrap();

        let message = ensure_no_incomplete_transaction(&install).unwrap_err();
        assert!(message.contains("记录格式无效"));
        assert!(incomplete_transaction(&install).is_err());
    }

    #[test]
    fn snapshot_restores_missing_paths_by_removing_new_files() {
        let install = temp_dir("snapshot");
        let mut transaction = new_transaction(&install);
        write_transaction(&install, &transaction).unwrap();
        snapshot_managed_files(&install, &mut transaction, &["deploy/new.conf".to_string()])
            .unwrap();
        assert_eq!(transaction.files.len(), 1);
        assert!(!transaction.files[0].existed);

        fs::create_dir_all(install.join("deploy")).unwrap();
        fs::write(install.join("deploy/new.conf"), "new").unwrap();
        restore_managed_files(&install, &transaction).unwrap();
        assert!(!install.join("deploy/new.conf").exists());
    }

    #[test]
    fn snapshot_rejects_parent_traversal() {
        let install = temp_dir("traversal");
        let mut transaction = new_transaction(&install);
        let error = snapshot_managed_files(&install, &mut transaction, &["../outside".to_string()])
            .unwrap_err();
        assert!(error.contains("无效路径"));
    }

    #[test]
    fn recovery_rejects_a_corrupted_file_snapshot_before_touching_target() {
        let install = temp_dir("snapshot-verify");
        fs::create_dir_all(install.join("deploy")).unwrap();
        fs::write(install.join("deploy/app.jar"), "original").unwrap();
        let mut transaction = new_transaction(&install);
        write_transaction(&install, &transaction).unwrap();
        snapshot_managed_files(&install, &mut transaction, &["deploy/app.jar".to_string()])
            .unwrap();
        verify_recovery_snapshot(&transaction).unwrap();

        fs::write(
            transaction.snapshot_path().join("files/deploy/app.jar"),
            "tampered",
        )
        .unwrap();
        let error = verify_recovery_snapshot(&transaction).unwrap_err();
        assert!(error.contains("完整性校验失败") || error.contains("大小校验失败"));
        assert_eq!(
            fs::read_to_string(install.join("deploy/app.jar")).unwrap(),
            "original"
        );
    }

    #[test]
    fn docker_volume_backup_round_trip_when_enabled() {
        let Ok(image) = std::env::var("DEPLOY_TEST_VOLUME_IMAGE") else {
            return;
        };
        let volume = format!(
            "deploy-desktop-tx-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let run = |script: &str| {
            let mut command = crate::commands::docker_setup::docker_command();
            command
                .args(["run", "--rm", "-v"])
                .arg(format!("{volume}:/data"))
                .args(["--entrypoint", "/bin/sh", &image, "-c", script])
                .output()
                .unwrap()
        };
        let created = std::process::Command::new("docker")
            .args(["volume", "create", &volume])
            .output()
            .unwrap();
        assert!(created.status.success());
        let written = run("mkdir -p /data && printf old > /data/value.txt");
        assert!(written.status.success());
        let backup = temp_dir("volume-backup").join("data.tar.gz");
        let (sha, bytes) = backup_volume_local(&image, &volume, &backup).unwrap();
        assert!(bytes > 0 && sha.len() == 64);

        let overwritten = run("printf new > /data/value.txt");
        assert!(overwritten.status.success());
        restore_volume_local(&image, &volume, &backup).unwrap();
        let restored = run("cat /data/value.txt");
        assert!(restored.status.success());
        assert_eq!(String::from_utf8_lossy(&restored.stdout), "old");
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &volume])
            .output();
    }
}
