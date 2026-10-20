use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

use tauri::{AppHandle, Emitter, Manager};

use super::registry_sources::{reference_from_source, source_access, source_credentials};
use super::remote_ssh::find_binary;

const SETTINGS_FILE: &str = "image-library-settings.json";
const MANIFEST_FILE: &str = "images.json";
const DEFAULT_DIR_NAME: &str = "离线部署助手";
const DEFAULT_PACKAGE_DIR: &str = "离线镜像包";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageLibrarySettings {
    pub offline_dir: String,
    pub default_dir: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ImageLibrarySettingsFile {
    offline_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDockerImage {
    pub reference: String,
    pub repository: String,
    pub tag: String,
    pub id: String,
    pub digest: String,
    pub size_bytes: u64,
    pub size_text: String,
    pub created_text: String,
    pub exportable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflinePackageInfo {
    pub name: String,
    pub size_bytes: u64,
    pub modified_ms: u64,
    pub image_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflinePackageScan {
    pub offline_dir: String,
    pub exists: bool,
    pub files: Vec<OfflinePackageInfo>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub target: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageExportResult {
    pub reference: String,
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageLibraryProgress {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub reference: String,
    pub percent: usize,
    pub message: String,
    pub finished: bool,
}

#[tauri::command]
pub fn get_image_library_settings(app: AppHandle) -> Result<ImageLibrarySettings, String> {
    Ok(ImageLibrarySettings {
        offline_dir: effective_offline_dir(&app).display().to_string(),
        default_dir: default_offline_dir(&app).display().to_string(),
    })
}

#[tauri::command]
pub fn save_image_library_settings(
    app: AppHandle,
    offline_dir: String,
) -> Result<ImageLibrarySettings, String> {
    let dir = normalize_offline_dir(&offline_dir)?;
    let settings = ImageLibrarySettingsFile {
        offline_dir: dir.display().to_string(),
    };
    let path = settings_path(&app)?;
    let content =
        serde_json::to_string_pretty(&settings).map_err(|e| format!("保存离线包目录失败：{e}"))?;
    fs::write(&path, content).map_err(|e| format!("保存离线包目录失败：{e}"))?;

    Ok(ImageLibrarySettings {
        offline_dir: dir.display().to_string(),
        default_dir: default_offline_dir(&app).display().to_string(),
    })
}

#[tauri::command]
pub fn list_local_docker_images() -> Result<Vec<LocalDockerImage>, String> {
    let output = Command::new("docker")
        .args(["image", "ls", "--no-trunc", "--format", "{{json .}}"])
        .output()
        .map_err(docker_command_error)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(docker_failure(&stderr));
    }

    let mut images = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(image) = parse_docker_image_line(line) {
            images.push(image);
        }
    }
    Ok(images)
}

fn parse_docker_image_line(line: &str) -> Option<LocalDockerImage> {
    let value = serde_json::from_str::<Value>(line).ok()?;
    let repository = value
        .get("Repository")
        .and_then(Value::as_str)
        .unwrap_or("<none>")
        .to_string();
    let tag = value
        .get("Tag")
        .and_then(Value::as_str)
        .unwrap_or("<none>")
        .to_string();
    let id = value
        .get("ID")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let digest = value
        .get("Digest")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let size_text = value
        .get("Size")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let created_text = value
        .get("CreatedAt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let untagged = repository == "<none>" || tag == "<none>";
    let reference = if untagged {
        id.clone()
    } else {
        format!("{repository}:{tag}")
    };
    if reference.is_empty() {
        return None;
    }
    Some(LocalDockerImage {
        reference,
        repository,
        tag,
        id,
        digest,
        size_bytes: parse_docker_size(&size_text),
        size_text,
        created_text,
        exportable: !untagged,
    })
}

#[tauri::command]
pub async fn pull_local_docker_image(
    window: tauri::Window,
    app: AppHandle,
    reference: String,
    username: Option<String>,
    password: Option<String>,
    registry_source_id: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let access = source_access(&app, registry_source_id.as_deref())?;
        let (reference, username, password) = if let Some(access) = access {
            let reference = reference_from_source(&access.source, &reference)?;
            let (username, password) = source_credentials(access, password)?;
            (reference, username, password)
        } else {
            (reference, username, password)
        };
        let reference = validate_pull_reference(&reference)?;
        let username = username.unwrap_or_default();
        let username = username.trim().to_string();
        let password = password.unwrap_or_default();
        let mut temp_config = None;
        if !username.is_empty() {
            if password.is_empty() {
                return Err("已填写仓库用户名，请同时填写密码".to_string());
            }
            let registry = registry_for_reference(&reference);
            let config = TempDockerConfig::create()?;
            emit_progress(
                &window,
                "pull",
                0,
                1,
                &reference,
                5,
                "正在登录镜像仓库",
                false,
            );
            login_docker(&registry, &username, &password, &config.path)?;
            temp_config = Some(config);
        } else if !password.is_empty() {
            return Err("已填写仓库密码，请同时填写仓库用户名".to_string());
        }

        emit_progress(&window, "pull", 1, 1, &reference, 20, "正在拉取镜像", false);
        let mut command = Command::new(find_binary("docker"));
        command.args(["pull", &reference]);
        if let Some(config) = &temp_config {
            command.env("DOCKER_CONFIG", &config.path);
        }
        let output = command.output().map_err(docker_command_error)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(docker_failure(&redact_secret(&stderr, &password)));
        }

        let message = redact_secret(&compact_docker_output(&output.stdout, &output.stderr), &password);
        emit_progress(
            &window,
            "pull",
            1,
            1,
            &reference,
            100,
            "镜像拉取完成",
            false,
        );
        Ok(if message.is_empty() {
            format!("已拉取 {reference}")
        } else {
            message
        })
    })
    .await
    .map_err(|e| format!("拉取镜像任务失败：{e}"))?
}

#[tauri::command]
pub async fn import_offline_packages(
    window: tauri::Window,
    app: AppHandle,
    files: Vec<String>,
) -> Result<Vec<OperationResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if files.is_empty() {
            return Err("请选择要导入的离线包".to_string());
        }
        let dir = effective_offline_dir(&app);
        let total = files.len();
        let mut results = Vec::with_capacity(total);

        for (index, raw) in files.iter().enumerate() {
            let name = match safe_package_name(raw) {
                Ok(value) => value,
                Err(error) => {
                    results.push(OperationResult {
                        target: raw.clone(),
                        success: false,
                        message: error,
                    });
                    continue;
                }
            };
            let path = dir.join(&name);
            if !path.is_file() {
                results.push(OperationResult {
                    target: name,
                    success: false,
                    message: "文件不存在，可能已被删除".to_string(),
                });
                continue;
            }

            emit_progress(
                &window,
                "import",
                index + 1,
                total,
                &name,
                0,
                &format!("正在导入 {name}"),
                false,
            );
            let output = Command::new("docker")
                .arg("load")
                .arg("-i")
                .arg(&path)
                .output();
            let result = match output {
                Ok(output) if output.status.success() => {
                    let message = compact_docker_output(&output.stdout, &output.stderr);
                    OperationResult {
                        target: name.clone(),
                        success: true,
                        message: if message.is_empty() {
                            "已导入".to_string()
                        } else {
                            message
                        },
                    }
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    OperationResult {
                        target: name.clone(),
                        success: false,
                        message: docker_failure(&stderr),
                    }
                }
                Err(error) => OperationResult {
                    target: name.clone(),
                    success: false,
                    message: docker_command_error(error),
                },
            };
            results.push(result);
            emit_progress(
                &window,
                "import",
                index + 1,
                total,
                &name,
                100,
                &format!("{name} 处理完成"),
                false,
            );
        }
        emit_progress(
            &window,
            "done",
            total,
            total,
            "ALL",
            100,
            "离线包导入完成",
            true,
        );
        Ok(results)
    })
    .await
    .map_err(|e| format!("导入离线包任务失败：{e}"))?
}

#[tauri::command]
pub async fn remove_local_docker_images(
    window: tauri::Window,
    references: Vec<String>,
) -> Result<Vec<OperationResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if references.is_empty() {
            return Err("请选择要删除的镜像".to_string());
        }
        let total = references.len();
        let mut results = Vec::with_capacity(total);
        for (index, raw) in references.iter().enumerate() {
            let reference = match validate_reference(raw) {
                Ok(value) => value,
                Err(error) => {
                    results.push(OperationResult {
                        target: raw.clone(),
                        success: false,
                        message: error,
                    });
                    continue;
                }
            };
            emit_progress(
                &window,
                "delete",
                index + 1,
                total,
                &reference,
                0,
                &format!("正在删除 {reference}"),
                false,
            );
            let result = match Command::new("docker").args(["rmi", &reference]).output() {
                Ok(output) if output.status.success() => OperationResult {
                    target: reference.clone(),
                    success: true,
                    message: "已删除".to_string(),
                },
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let mut message = docker_failure(&stderr);
                    if message.contains("image is being used by") || message.contains("container") {
                        message = format!("{message}（该镜像仍被容器使用，需先移除相关容器）");
                    }
                    OperationResult {
                        target: reference.clone(),
                        success: false,
                        message,
                    }
                }
                Err(error) => OperationResult {
                    target: reference.clone(),
                    success: false,
                    message: docker_command_error(error),
                },
            };
            results.push(result);
            emit_progress(
                &window,
                "delete",
                index + 1,
                total,
                &reference,
                100,
                &format!("{reference} 处理完成"),
                false,
            );
        }
        emit_progress(
            &window,
            "done",
            total,
            total,
            "ALL",
            100,
            "镜像删除完成",
            true,
        );
        Ok(results)
    })
    .await
    .map_err(|e| format!("删除镜像任务失败：{e}"))?
}

#[tauri::command]
pub async fn export_local_docker_images(
    window: tauri::Window,
    app: AppHandle,
    references: Vec<String>,
) -> Result<Vec<ImageExportResult>, String> {
    tauri::async_runtime::spawn_blocking(move || export_images_blocking(&window, &app, references))
        .await
        .map_err(|e| format!("导出离线包任务失败：{e}"))?
}

#[tauri::command]
pub fn scan_offline_packages(app: AppHandle) -> Result<OfflinePackageScan, String> {
    scan_offline_dir(&effective_offline_dir(&app))
}

#[tauri::command]
pub fn remove_offline_packages(
    app: AppHandle,
    files: Vec<String>,
) -> Result<Vec<OperationResult>, String> {
    if files.is_empty() {
        return Err("请选择要删除的离线包".to_string());
    }
    let dir = effective_offline_dir(&app);
    let mut results = Vec::with_capacity(files.len());
    let mut removed = HashSet::new();

    for raw in files {
        let name = match safe_package_name(&raw) {
            Ok(value) => value,
            Err(error) => {
                results.push(OperationResult {
                    target: raw,
                    success: false,
                    message: error,
                });
                continue;
            }
        };
        let path = dir.join(&name);
        match fs::remove_file(&path) {
            Ok(()) => {
                removed.insert(name.clone());
                results.push(OperationResult {
                    target: name,
                    success: true,
                    message: "已删除".to_string(),
                });
            }
            Err(error) => {
                let message = if error.kind() == ErrorKind::NotFound {
                    "文件不存在，可能已被删除".to_string()
                } else {
                    format!("删除失败：{error}")
                };
                results.push(OperationResult {
                    target: name,
                    success: false,
                    message,
                });
            }
        }
    }

    if !removed.is_empty() {
        remove_manifest_entries(&dir, &removed);
    }
    Ok(results)
}

fn default_offline_dir(app: &AppHandle) -> PathBuf {
    let base = app
        .path()
        .document_dir()
        .ok()
        .filter(|path| path.is_absolute())
        .or_else(dirs::document_dir)
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(DEFAULT_DIR_NAME).join(DEFAULT_PACKAGE_DIR)
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("读取应用配置目录失败：{e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建应用配置目录失败：{e}"))?;
    Ok(dir.join(SETTINGS_FILE))
}

fn read_settings(app: &AppHandle) -> ImageLibrarySettingsFile {
    let Ok(path) = settings_path(app) else {
        return ImageLibrarySettingsFile::default();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return ImageLibrarySettingsFile::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn effective_offline_dir(app: &AppHandle) -> PathBuf {
    let settings = read_settings(app);
    let saved = settings.offline_dir.trim();
    if saved.is_empty() {
        default_offline_dir(app)
    } else {
        absolute_path(saved)
    }
}

fn normalize_offline_dir(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("请填写离线包保存目录".to_string());
    }
    let path = absolute_path(trimmed);
    fs::create_dir_all(&path)
        .map_err(|e| format!("创建离线包目录失败：{}（{e}）", path.display()))?;
    if !path.is_dir() {
        return Err(format!("离线包保存目录不是文件夹：{}", path.display()));
    }
    Ok(path)
}

fn absolute_path(raw: &str) -> PathBuf {
    let path = expand_path(raw);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn expand_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

fn docker_command_error(error: std::io::Error) -> String {
    if error.kind() == ErrorKind::NotFound {
        "未找到 Docker 命令，请先安装 Docker Desktop 并重新打开应用".to_string()
    } else {
        format!("启动 Docker 命令失败：{error}")
    }
}

fn docker_failure(stderr: &str) -> String {
    let text = stderr.trim();
    if text.is_empty() {
        return "Docker 命令执行失败，未返回错误信息".to_string();
    }
    let lower = text.to_lowercase();
    if lower.contains("cannot connect to the docker daemon")
        || lower.contains("is the docker daemon running")
        || lower.contains("permission denied while trying to connect")
        || lower.contains("error during connect")
    {
        return format!("Docker 服务不可用，请确认 Docker 已启动且当前用户可访问 Docker。{text}");
    }
    format!("Docker 命令执行失败：{text}")
}

fn compact_docker_output(stdout: &[u8], stderr: &[u8]) -> String {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    )
    .replace('\r', "\n");
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn validate_reference(reference: &str) -> Result<String, String> {
    let value = reference.trim();
    if value.is_empty() {
        return Err("请填写镜像地址".to_string());
    }
    if value.starts_with('-') {
        return Err("镜像地址不能以 - 开头".to_string());
    }
    if value.len() > 512 {
        return Err("镜像地址过长".to_string());
    }
    if value
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err("镜像地址不能包含空格或控制字符".to_string());
    }
    Ok(value.to_string())
}

fn validate_pull_reference(reference: &str) -> Result<String, String> {
    let value = validate_reference(reference)?;
    let Some((repository, digest)) = value.rsplit_once("@sha256:") else {
        return Err("请使用包含 sha256 摘要的镜像地址，以固定镜像版本".to_string());
    };
    if repository.is_empty() || repository.starts_with('/') || repository.contains("..")
        || repository.contains("//") || repository.contains('@') || repository.contains("://")
        || digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("镜像地址无效，请检查仓库路径及 64 位小写 sha256 摘要".to_string());
    }
    Ok(value)
}

fn redact_secret(text: &str, password: &str) -> String {
    if password.is_empty() { text.to_string() } else { text.replace(password, "[已隐藏]") }
}

fn registry_for_reference(reference: &str) -> String {
    let first = reference.split('/').next().unwrap_or("");
    if reference.contains('/')
        && (first.contains('.') || first.contains(':') || first == "localhost")
    {
        first.to_string()
    } else {
        "docker.io".to_string()
    }
}

pub(super) fn login_docker(
    registry: &str,
    username: &str,
    password: &str,
    docker_config: &Path,
) -> Result<(), String> {
    let mut command = Command::new(find_binary("docker"));
    command
        .args(["login", registry, "-u", username, "--password-stdin"])
        .env("DOCKER_CONFIG", docker_config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(docker_command_error)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(password.as_bytes())
            .map_err(|e| format!("写入仓库密码失败：{e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 Docker 登录失败：{e}"))?;
    if !output.status.success() {
        return Err("镜像源登录失败，请检查用户名、密码、网络和仓库证书".to_string());
    }
    Ok(())
}

pub(super) struct TempDockerConfig {
    pub(super) path: PathBuf,
}

impl TempDockerConfig {
    pub(super) fn create() -> Result<Self, String> {
        let stamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "deploy-desktop-docker-config-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).map_err(|e| format!("创建临时 Docker 配置目录失败：{e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o700));
        }
        Ok(Self { path })
    }
}

impl Drop for TempDockerConfig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn parse_docker_size(text: &str) -> u64 {
    let value = text.trim();
    if value.is_empty() {
        return 0;
    }
    let index = value
        .find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .unwrap_or(value.len());
    let number: f64 = value[..index].parse().unwrap_or(0.0);
    let unit = value[index..].trim().to_ascii_uppercase();
    let multiplier = match unit.as_str() {
        "" | "B" => 1.0,
        "KB" => 1_000.0,
        "MB" => 1_000_000.0,
        "GB" => 1_000_000_000.0,
        "TB" => 1_000_000_000_000.0,
        "PB" => 1_000_000_000_000_000.0,
        "KIB" => 1024.0,
        "MIB" => 1024.0 * 1024.0,
        "GIB" => 1024.0 * 1024.0 * 1024.0,
        "TIB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 0.0,
    };
    (number * multiplier) as u64
}

fn export_images_blocking(
    window: &tauri::Window,
    app: &AppHandle,
    references: Vec<String>,
) -> Result<Vec<ImageExportResult>, String> {
    if references.is_empty() {
        return Err("请选择要导出的镜像".to_string());
    }
    let mut seen = HashSet::new();
    let references: Vec<String> = references
        .into_iter()
        .map(|reference| reference.trim().to_string())
        .filter(|reference| !reference.is_empty() && seen.insert(reference.clone()))
        .collect();
    if references.is_empty() {
        return Err("请选择要导出的镜像".to_string());
    }

    let dir = effective_offline_dir(app);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("创建离线包目录失败：{}（{e}）", dir.display()))?;
    let manifest = read_manifest(&dir);
    let manifest_map = manifest_file_map(&manifest);
    let mut existing_names = existing_package_names(&dir);
    let total = references.len();
    let mut results = Vec::with_capacity(total);
    let mut exports = Vec::new();

    for (index, raw) in references.iter().enumerate() {
        let reference = match validate_reference(raw) {
            Ok(value) => value,
            Err(error) => {
                results.push(ImageExportResult {
                    reference: raw.clone(),
                    file_name: String::new(),
                    path: String::new(),
                    size_bytes: 0,
                    success: false,
                    message: error,
                });
                continue;
            }
        };
        emit_progress(
            window,
            "export",
            index + 1,
            total,
            &reference,
            0,
            &format!("正在导出 {reference}"),
            false,
        );

        let file_name = unique_archive_name(&reference, &manifest_map, &existing_names);
        existing_names.insert(file_name.clone());
        let target = dir.join(&file_name);
        let temp = dir.join(format!(".{file_name}.part"));
        let _ = fs::remove_file(&temp);

        let output = Command::new("docker")
            .arg("save")
            .arg("--output")
            .arg(&temp)
            .arg(&reference)
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let _ = fs::remove_file(&target);
                if let Err(error) = fs::rename(&temp, &target) {
                    let _ = fs::remove_file(&temp);
                    results.push(ImageExportResult {
                        reference: reference.clone(),
                        file_name,
                        path: target.display().to_string(),
                        size_bytes: 0,
                        success: false,
                        message: format!("保存离线包失败：{error}"),
                    });
                    continue;
                }
                let size = fs::metadata(&target).map(|meta| meta.len()).unwrap_or(0);
                exports.push((reference.clone(), file_name.clone()));
                results.push(ImageExportResult {
                    reference: reference.clone(),
                    file_name: file_name.clone(),
                    path: target.display().to_string(),
                    size_bytes: size,
                    success: true,
                    message: format!("已导出 {file_name}"),
                });
                emit_progress(
                    window,
                    "export",
                    index + 1,
                    total,
                    &reference,
                    100,
                    &format!("{file_name} 导出完成"),
                    false,
                );
            }
            Ok(output) => {
                let _ = fs::remove_file(&temp);
                let stderr = String::from_utf8_lossy(&output.stderr);
                results.push(ImageExportResult {
                    reference: reference.clone(),
                    file_name,
                    path: target.display().to_string(),
                    size_bytes: 0,
                    success: false,
                    message: docker_failure(&stderr),
                });
            }
            Err(error) => {
                let _ = fs::remove_file(&temp);
                results.push(ImageExportResult {
                    reference: reference.clone(),
                    file_name,
                    path: target.display().to_string(),
                    size_bytes: 0,
                    success: false,
                    message: docker_command_error(error),
                });
            }
        }
    }

    if !exports.is_empty() {
        if let Err(error) = update_manifest_for_exports(&dir, &exports) {
            log::warn!("更新离线镜像清单失败：{error}");
        }
    }
    emit_progress(
        window,
        "done",
        total,
        total,
        "ALL",
        100,
        "离线包导出完成",
        true,
    );
    Ok(results)
}

fn existing_package_names(dir: &Path) -> HashSet<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_file() && is_image_file(&entry.path()))
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn unique_archive_name(
    reference: &str,
    manifest_map: &HashMap<String, Vec<String>>,
    existing_names: &HashSet<String>,
) -> String {
    let base = archive_name_for(reference);
    if !existing_names.contains(&base) {
        return base;
    }
    let same_reference = manifest_map
        .get(&base)
        .map(|images| images.len() == 1 && images[0] == reference)
        .unwrap_or(false);
    if same_reference {
        return base;
    }

    let stem = base.strip_suffix(".tar").unwrap_or(&base);
    let mut index = 2;
    loop {
        let candidate = format!("{stem}-{index}.tar");
        if !existing_names.contains(&candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn archive_name_for(reference: &str) -> String {
    let mut safe: String = reference
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    safe = safe.trim_matches('.').to_string();
    if safe.is_empty() {
        safe = "image".to_string();
    }
    if safe.chars().count() > 120 {
        safe = safe.chars().take(96).collect();
    }
    let lower = safe.to_ascii_lowercase();
    if lower.ends_with(".tar") {
        safe
    } else {
        format!("{safe}.tar")
    }
}

fn read_manifest(dir: &Path) -> Value {
    let path = dir.join(MANIFEST_FILE);
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_else(|| json!({"schemaVersion": 1, "files": []}))
}

fn manifest_file_map(manifest: &Value) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    if let Some(files) = manifest.get("files").and_then(Value::as_array) {
        for entry in files {
            let Some(file) = entry.get("file").and_then(Value::as_str) else {
                continue;
            };
            let images = entry
                .get("images")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            map.insert(file.to_string(), images);
        }
    }
    map
}

fn update_manifest_for_exports(dir: &Path, exports: &[(String, String)]) -> Result<(), String> {
    if exports.is_empty() {
        return Ok(());
    }
    let mut root = read_manifest(dir);
    if !root.is_object() {
        root = json!({});
    }
    let object = root.as_object_mut().expect("manifest should be an object");
    object.entry("schemaVersion").or_insert(json!(1));

    let exported_refs: HashSet<&str> = exports
        .iter()
        .map(|(reference, _)| reference.as_str())
        .collect();
    let exported_files: HashSet<&str> = exports.iter().map(|(_, file)| file.as_str()).collect();
    let files = object
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut filtered = Vec::new();

    for entry in files {
        let Some(file) = entry.get("file").and_then(Value::as_str) else {
            continue;
        };
        if exported_files.contains(file) {
            continue;
        }
        let original_images: Vec<String> = entry
            .get("images")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let remaining: Vec<String> = original_images
            .iter()
            .filter(|image| !exported_refs.contains(image.as_str()))
            .cloned()
            .collect();
        if remaining.len() != original_images.len() {
            if remaining.is_empty() {
                continue;
            }
            let mut entry = entry;
            entry["images"] = json!(remaining);
            filtered.push(entry);
        } else {
            filtered.push(entry);
        }
    }

    for (reference, file) in exports {
        filtered.push(json!({"file": file, "images": [reference]}));
    }
    object.insert("files".to_string(), Value::Array(filtered));
    write_manifest(dir, &root)
}

fn remove_manifest_entries(dir: &Path, names: &HashSet<String>) {
    let path = dir.join(MANIFEST_FILE);
    if !path.is_file() {
        return;
    }
    let mut root = read_manifest(dir);
    let Some(object) = root.as_object_mut() else {
        return;
    };
    let files = object
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let filtered: Vec<Value> = files
        .into_iter()
        .filter(|entry| {
            entry
                .get("file")
                .and_then(Value::as_str)
                .map(|file| !names.contains(file))
                .unwrap_or(true)
        })
        .collect();
    object.insert("files".to_string(), Value::Array(filtered));
    if let Err(error) = write_manifest(dir, &root) {
        log::warn!("更新离线镜像清单失败：{error}");
    }
}

fn write_manifest(dir: &Path, root: &Value) -> Result<(), String> {
    let path = dir.join(MANIFEST_FILE);
    let temp = dir.join(".images.json.tmp");
    let content =
        serde_json::to_string_pretty(root).map_err(|e| format!("生成镜像清单失败：{e}"))?;
    fs::write(&temp, content).map_err(|e| format!("写入镜像清单失败：{e}"))?;
    match fs::rename(&temp, &path) {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = fs::remove_file(&path);
            fs::rename(&temp, &path).map_err(|e| format!("更新镜像清单失败：{e}"))
        }
    }
}

fn scan_offline_dir(dir: &Path) -> Result<OfflinePackageScan, String> {
    let manifest = read_manifest(dir);
    let manifest_map = manifest_file_map(&manifest);
    if !dir.is_dir() {
        return Ok(OfflinePackageScan {
            offline_dir: dir.display().to_string(),
            exists: false,
            files: Vec::new(),
            total_bytes: 0,
        });
    }

    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !is_image_file(&path) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().to_string();
            total_bytes += metadata.len();
            files.push(OfflinePackageInfo {
                image_refs: manifest_map.get(&name).cloned().unwrap_or_default(),
                name,
                size_bytes: metadata.len(),
                modified_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0),
            });
        }
    }
    files.sort_by(|left, right| {
        right
            .modified_ms
            .cmp(&left.modified_ms)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(OfflinePackageScan {
        offline_dir: dir.display().to_string(),
        exists: true,
        files,
        total_bytes,
    })
}

fn is_image_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    name.ends_with(".tar") || name.ends_with(".tgz") || name.ends_with(".tar.gz")
}

fn safe_package_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed == "."
        || trimmed == ".."
    {
        return Err(format!("非法的离线包文件名：{name}"));
    }
    let lower = trimmed.to_ascii_lowercase();
    if !lower.ends_with(".tar") && !lower.ends_with(".tgz") && !lower.ends_with(".tar.gz") {
        return Err("只能删除 .tar、.tgz 或 .tar.gz 离线包".to_string());
    }
    Ok(trimmed.to_string())
}

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    window: &tauri::Window,
    phase: &str,
    current: usize,
    total: usize,
    reference: &str,
    percent: usize,
    message: &str,
    finished: bool,
) {
    let _ = window.emit(
        "image-library-progress",
        ImageLibraryProgress {
            phase: phase.to_string(),
            current,
            total,
            reference: reference.to_string(),
            percent,
            message: message.to_string(),
            finished,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deploy-desktop-image-library-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn archive_names_are_safe_and_end_with_tar() {
        assert_eq!(
            archive_name_for("registry.example.com/demo/app:1.0.0"),
            "registry.example.com_demo_app_1.0.0.tar"
        );
        assert_eq!(archive_name_for("../../etc/passwd"), "_.._etc_passwd.tar");
        assert!(!archive_name_for("a/b:c").contains('/'));
    }

    #[test]
    fn pulls_require_a_fixed_digest() {
        let digest = "a".repeat(64);
        assert!(validate_pull_reference(&format!("registry.example.com/demo/app@sha256:{digest}")).is_ok());
        assert!(validate_pull_reference("demo/app:latest").is_err());
        assert!(validate_pull_reference("demo/app@sha256:abc").is_err());
        assert!(validate_pull_reference(&format!("https://registry.example.com/app@sha256:{digest}")).is_err());
    }

    #[test]
    fn docker_sizes_are_parsed() {
        assert_eq!(parse_docker_size("145MB"), 145_000_000);
        assert_eq!(parse_docker_size("1.2GB"), 1_200_000_000);
        assert_eq!(parse_docker_size("512MiB"), 512 * 1024 * 1024);
        assert_eq!(parse_docker_size("0B"), 0);
    }

    #[test]
    fn docker_image_rows_are_parsed() {
        let tagged = parse_docker_image_line(
            r#"{"Containers":"0","CreatedAt":"2026-10-19 23:23:30 +0800 CST","Digest":"\u003cnone\u003e","ID":"sha256:ca1a","Repository":"demo/app","SharedSize":"N/A","Size":"55.6MB","Tag":"latest","UniqueSize":"N/A"}"#,
        )
        .unwrap();
        assert_eq!(tagged.reference, "demo/app:latest");
        assert_eq!(tagged.size_bytes, 55_600_000);
        assert!(tagged.exportable);

        let untagged = parse_docker_image_line(
            r#"{"CreatedAt":"2026-10-19 23:23:30 +0800 CST","Digest":"<none>","ID":"sha256:deadbeef","Repository":"<none>","Size":"12MB","Tag":"<none>"}"#,
        )
        .unwrap();
        assert_eq!(untagged.reference, "sha256:deadbeef");
        assert!(!untagged.exportable);
        assert_eq!(untagged.size_text, "12MB");
    }

    #[test]
    fn package_names_reject_traversal_and_non_images() {
        assert!(safe_package_name("../app.tar").is_err());
        assert!(safe_package_name("images.json").is_err());
        assert_eq!(safe_package_name("app.tar").unwrap(), "app.tar");
    }

    #[test]
    fn manifest_scan_maps_packages_to_images() {
        let dir = temp_dir("scan");
        fs::write(dir.join("app.tar"), b"image").unwrap();
        fs::write(
            dir.join(MANIFEST_FILE),
            r#"{"schemaVersion":1,"files":[{"file":"app.tar","images":["demo/app:1"]}]}"#,
        )
        .unwrap();

        let scan = scan_offline_dir(&dir).unwrap();
        assert!(scan.exists);
        assert_eq!(scan.files.len(), 1);
        assert_eq!(scan.files[0].image_refs, vec!["demo/app:1"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exporting_one_image_keeps_other_manifest_entries() {
        let dir = temp_dir("manifest");
        fs::write(
            dir.join(MANIFEST_FILE),
            r#"{"schemaVersion":1,"files":[
                {"file":"stack.tar.gz","images":["demo/backend:1","demo/gateway:1"]},
                {"file":"app.tar","images":["demo/app:1"]}
            ]}"#,
        )
        .unwrap();

        update_manifest_for_exports(
            &dir,
            &[("demo/backend:1".to_string(), "backend.tar".to_string())],
        )
        .unwrap();

        let manifest = read_manifest(&dir);
        let map = manifest_file_map(&manifest);
        assert_eq!(map.get("stack.tar.gz").unwrap(), &vec!["demo/gateway:1"]);
        assert_eq!(map.get("app.tar").unwrap(), &vec!["demo/app:1"]);
        assert_eq!(map.get("backend.tar").unwrap(), &vec!["demo/backend:1"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_names_keep_existing_untracked_files() {
        let dir = temp_dir("unique");
        fs::write(dir.join("app.tar"), b"x").unwrap();
        let existing = existing_package_names(&dir);
        let manifest = HashMap::new();
        assert_eq!(
            unique_archive_name("app", &manifest, &existing),
            "app-2.tar"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
