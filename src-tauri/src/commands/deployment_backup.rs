//! 部署备份盘点与清理。
//!
//! 增量更新会把被替换的文件复制到 `<部署目录>/backup/update-*`；完整版本部署在
//! 成功提交或恢复完成后也可能留下 `<部署目录>/.deploy-desktop/transactions/<id>/snapshot`。
//! 两类目录只用于回滚，随部署次数累积会持续占用磁盘。
//!
//! 清理范围严格限制在部署目录内，并且只处理部署代码已经不再需要的对象：
//! - `backup/update-*`：更新前的文件副本；
//! - 状态为 `committed` 或 `recovered` 的事务快照。
//!
//! 运行中、失败待恢复、恢复中或恢复失败的事务不会被删除，避免破坏回滚现场。

use crate::commands::{deployment_tx, remote_ssh, version_deploy};
use crate::core::{backup, path_guard, shell};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const BACKUP_DIR: &str = "backup";
const UPDATE_PREFIX: &str = "update-";
const TRANSACTION_PREFIX: &str = "deploy-";
const SNAPSHOT_DIR: &str = "snapshot";
const JOURNAL_FILE: &str = "journal.json";
const BACKUP_META_DIR: &str = ".meta";
const UPDATING_MARKER: &str = ".updating";
const UPDATING_STALE_SECS: u64 = 6 * 60 * 60;
const REMOTE_CLEANUP_TIMEOUT_SECS: u64 = 600;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentBackupEntry {
    pub name: String,
    pub kind: String,
    /// 备份生成时部署目录中的版本；旧备份没有版本标记时为空。
    pub version: String,
    pub bytes: u64,
    pub modified_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentBackupSelection {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentBackupOverview {
    pub current_version: String,
    pub update_backups: Vec<DeploymentBackupEntry>,
    pub transaction_snapshots: Vec<DeploymentBackupEntry>,
    pub update_bytes: u64,
    pub transaction_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentBackupCleanup {
    pub removed: usize,
    pub freed_bytes: u64,
    pub skipped: usize,
}

#[tauri::command]
pub async fn inspect_deployment_backups(
    install_dir: String,
) -> Result<DeploymentBackupOverview, String> {
    crate::commands::blocking::run(move || inspect_local(&install_dir)).await
}

#[tauri::command]
pub async fn clean_deployment_backups(
    install_dir: String,
    selections: Vec<DeploymentBackupSelection>,
) -> Result<DeploymentBackupCleanup, String> {
    crate::commands::blocking::run(move || clean_local(&install_dir, selections)).await
}

#[tauri::command]
pub async fn inspect_remote_deployment_backups(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    install_dir: String,
) -> Result<DeploymentBackupOverview, String> {
    crate::commands::blocking::run(move || {
        inspect_remote(&host, port, &user, key_path, &install_dir)
    })
    .await
}

#[tauri::command]
pub async fn clean_remote_deployment_backups(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    install_dir: String,
    selections: Vec<DeploymentBackupSelection>,
) -> Result<DeploymentBackupCleanup, String> {
    crate::commands::blocking::run(move || {
        clean_remote(&host, port, &user, key_path, &install_dir, selections)
    })
    .await
}

fn inspect_local(install_dir: &str) -> Result<DeploymentBackupOverview, String> {
    let install = version_deploy::resolve_install_root(install_dir, true)?;
    let update_backups = local_update_backups(&install)?;
    let transaction_snapshots = local_transaction_snapshots(&install)?;
    Ok(overview(
        update_backups,
        transaction_snapshots,
        deployed_version(&install),
    ))
}

fn clean_local(
    install_dir: &str,
    selections: Vec<DeploymentBackupSelection>,
) -> Result<DeploymentBackupCleanup, String> {
    if selections.is_empty() {
        return Err("请选择要删除的备份".to_string());
    }
    let install = version_deploy::resolve_install_root(install_dir, true)?;
    let _lock = deployment_tx::DeploymentLock::acquire(&install)?;
    let update_backups = local_update_backups(&install)?;
    let transaction_snapshots = local_transaction_snapshots(&install)?;

    let mut result = DeploymentBackupCleanup {
        removed: 0,
        freed_bytes: 0,
        skipped: 0,
    };
    let mut seen = std::collections::HashSet::new();
    for selection in selections {
        let kind = selection.kind.trim();
        let name = selection.name.trim();
        if !seen.insert((kind.to_string(), name.to_string())) {
            continue;
        }
        let target = match kind {
            "update" => update_backups
                .iter()
                .find(|entry| entry.name == name)
                .map(|_| {
                    (
                        install.join(BACKUP_DIR),
                        install.join(BACKUP_DIR).join(name),
                    )
                }),
            "transaction" => transaction_snapshots
                .iter()
                .find(|entry| entry.name == name)
                .map(|_| {
                    (
                        install.join(deployment_tx::TRANSACTIONS_DIR),
                        install
                            .join(deployment_tx::TRANSACTIONS_DIR)
                            .join(name)
                            .join(SNAPSHOT_DIR),
                    )
                }),
            _ => None,
        };
        let Some((base, path)) = target else {
            result.skipped += 1;
            continue;
        };
        if kind == "update" {
            ensure_backup_meta_dir(&install)?;
            ensure_backup_meta_entry(&install, name)?;
        }
        remove_checked(&base, &path, name, &mut result).map_err(|error| {
            format!(
                "{error}；已清理 {} 项，释放 {} 字节",
                result.removed, result.freed_bytes
            )
        })?;
        if kind == "update" {
            remove_backup_meta(&install, name)?;
        }
    }
    Ok(result)
}

fn ensure_backup_meta_dir(install: &Path) -> Result<bool, String> {
    ensure_real_dir(
        &install.join(BACKUP_DIR).join(BACKUP_META_DIR),
        "备份版本目录",
    )
}

fn ensure_backup_meta_entry(install: &Path, name: &str) -> Result<(), String> {
    if !ensure_backup_meta_dir(install)? {
        return Ok(());
    }
    let path = install
        .join(BACKUP_DIR)
        .join(BACKUP_META_DIR)
        .join(format!("{name}.json"));
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => Ok(()),
        Ok(_) => Err(format!("备份 {name} 的版本信息不是普通文件，已保留备份")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("读取备份 {name} 的版本信息失败：{error}")),
    }
}

fn remove_backup_meta(install: &Path, name: &str) -> Result<(), String> {
    if !ensure_backup_meta_dir(install)? {
        return Ok(());
    }
    let path = install
        .join(BACKUP_DIR)
        .join(BACKUP_META_DIR)
        .join(format!("{name}.json"));
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            fs::remove_file(&path)
                .map_err(|error| format!("删除备份 {name} 的版本信息失败：{error}"))
        }
        Ok(_) => Err(format!("备份 {name} 的版本信息不是普通文件，已保留")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("读取备份 {name} 的版本信息失败：{error}")),
    }
}

fn remove_checked(
    base: &Path,
    target: &Path,
    name: &str,
    result: &mut DeploymentBackupCleanup,
) -> Result<(), String> {
    if !path_guard::path_is_within(base, target) {
        return Err(format!("备份路径超出部署目录，已停止清理：{name}"));
    }
    ensure_no_symlink_chain(base, target, name)?;
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            result.skipped += 1;
            return Ok(());
        }
        Err(error) => return Err(format!("读取备份 {name} 失败：{error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("备份 {name} 不是普通目录，已停止清理"));
    }
    let bytes = tree_bytes(target)?;
    backup::remove_path(target).map_err(|error| format!("清理备份 {name} 失败：{error}"))?;
    result.removed += 1;
    result.freed_bytes = result.freed_bytes.saturating_add(bytes);
    Ok(())
}

fn inspect_remote(
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<String>,
    install_dir: &str,
) -> Result<DeploymentBackupOverview, String> {
    let install = validate_remote_install(install_dir)?;
    let key = remote_ssh::resolve_existing_key(key_path, host)?;
    let script = remote_scan_script(&install);
    let output = remote_ssh::run_ssh_script(host, port, user, key.as_deref(), &script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(remote_error("读取目标机部署备份失败", &stderr));
    }
    let (update_backups, transaction_snapshots, current_version) =
        parse_remote_entries(&String::from_utf8_lossy(&output.stdout))?;
    Ok(overview(
        update_backups,
        transaction_snapshots,
        current_version,
    ))
}

fn clean_remote(
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<String>,
    install_dir: &str,
    selections: Vec<DeploymentBackupSelection>,
) -> Result<DeploymentBackupCleanup, String> {
    if selections.is_empty() {
        return Err("请选择要删除的备份".to_string());
    }
    let install = validate_remote_install(install_dir)?;
    let key = remote_ssh::resolve_existing_key(key_path, host)?;
    let scan = remote_scan_script(&install);
    let output = remote_ssh::run_ssh_script(host, port, user, key.as_deref(), &scan)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(remote_error("读取目标机部署备份失败", &stderr));
    }
    let (update_backups, transaction_snapshots, _) =
        parse_remote_entries(&String::from_utf8_lossy(&output.stdout))?;

    let mut update_targets = Vec::new();
    let mut transaction_targets = Vec::new();
    let mut selected_entries: Vec<(&DeploymentBackupEntry, String)> = Vec::new();
    let mut skipped = 0usize;
    let mut seen = std::collections::HashSet::new();
    for selection in selections {
        let kind = selection.kind.trim();
        let name = selection.name.trim();
        if !seen.insert((kind.to_string(), name.to_string())) {
            continue;
        }
        let entry = match kind {
            "update" => update_backups.iter().find(|entry| entry.name == name),
            "transaction" => transaction_snapshots
                .iter()
                .find(|entry| entry.name == name),
            _ => None,
        };
        let Some(entry) = entry else {
            skipped += 1;
            continue;
        };
        if kind == "update" {
            update_targets.push(format!("{install}/{BACKUP_DIR}/{}", entry.name));
        } else {
            transaction_targets.push(format!(
                "{install}/{}/{}",
                deployment_tx::TRANSACTIONS_DIR,
                entry.name
            ));
        }
        selected_entries.push((entry, kind.to_string()));
    }
    if update_targets.is_empty() && transaction_targets.is_empty() {
        return Ok(DeploymentBackupCleanup {
            removed: 0,
            freed_bytes: 0,
            skipped,
        });
    }

    let script = remote_cleanup_script(&install, &update_targets, &transaction_targets);
    let output = remote_ssh::run_ssh_script_with_timeout(
        host,
        port,
        user,
        key.as_deref(),
        &script,
        std::time::Duration::from_secs(REMOTE_CLEANUP_TIMEOUT_SECS),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(remote_error("清理目标机部署备份未完成", &stderr));
    }

    // 脚本逐项报告结果，避免用过滤后的列表把“跳过”误算成已删除。
    let mut results = std::collections::HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split('\t');
        if parts.next() != Some("RESULT") {
            continue;
        }
        let Some(state) = parts.next() else {
            continue;
        };
        let Some(kind) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        results.insert(
            (kind.to_string(), name.to_string()),
            state.eq_ignore_ascii_case("removed"),
        );
    }
    let mut removed = 0usize;
    let mut freed_bytes = 0u64;
    for (entry, kind) in selected_entries {
        if results.get(&(kind.clone(), entry.name.clone())) == Some(&true) {
            removed += 1;
            freed_bytes = freed_bytes.saturating_add(entry.bytes);
        } else {
            skipped += 1;
        }
    }
    Ok(DeploymentBackupCleanup {
        removed,
        freed_bytes,
        skipped,
    })
}

fn overview(
    update_backups: Vec<DeploymentBackupEntry>,
    transaction_snapshots: Vec<DeploymentBackupEntry>,
    current_version: String,
) -> DeploymentBackupOverview {
    let update_bytes = update_backups.iter().map(|entry| entry.bytes).sum();
    let transaction_bytes = transaction_snapshots.iter().map(|entry| entry.bytes).sum();
    DeploymentBackupOverview {
        current_version,
        update_backups,
        transaction_snapshots,
        update_bytes,
        transaction_bytes,
        total_bytes: update_bytes.saturating_add(transaction_bytes),
    }
}

/// 目标自身及其在部署目录内的每一级父目录都必须是真实目录。
///
/// 只检查目标本身不足以阻止 `backup -> /外部目录` 这类链接：`read_dir` 会跟随
/// 父目录链接，后续删除就会落到部署目录之外。
fn ensure_no_symlink_chain(base: &Path, target: &Path, name: &str) -> Result<(), String> {
    if !path_guard::path_is_within(base, target) {
        return Err(format!("备份路径超出部署目录，已停止清理：{name}"));
    }
    for ancestor in target.ancestors() {
        if !path_guard::path_is_within(base, ancestor) {
            break;
        }
        let metadata = match fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => return Err(format!("读取备份路径失败：{error}")),
        };
        if metadata.file_type().is_symlink() {
            return Err(format!("备份路径包含符号链接，已停止清理：{name}"));
        }
        if ancestor == base {
            break;
        }
    }
    Ok(())
}

/// 目录不存在时返回 `false`；存在但不是真实目录（包括符号链接）时报错。
fn ensure_real_dir(path: &Path, label: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!("{label}不能是符号链接"));
            }
            if !metadata.is_dir() {
                return Err(format!("{label}不是目录"));
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("读取{label}失败：{error}")),
    }
}

fn local_update_backups(install: &Path) -> Result<Vec<DeploymentBackupEntry>, String> {
    let root = install.join(BACKUP_DIR);
    if !ensure_real_dir(&root, "备份目录")? {
        return Ok(Vec::new());
    }
    ensure_real_dir(&root.join(BACKUP_META_DIR), "备份版本目录")?;
    let directory = fs::read_dir(&root).map_err(|error| format!("读取备份目录失败：{error}"))?;
    let mut entries = Vec::new();
    for entry in directory {
        let entry = entry.map_err(|error| format!("读取备份目录失败：{error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !valid_update_backup_name(&name) {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| format!("读取备份目录失败：{error}"))?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if backup_is_updating(&path) {
            continue;
        }
        entries.push(DeploymentBackupEntry {
            version: read_update_backup_version(install, &name),
            name,
            kind: "update".to_string(),
            bytes: tree_bytes(&path)?,
            modified_at: modified_ms(&path),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn local_transaction_snapshots(install: &Path) -> Result<Vec<DeploymentBackupEntry>, String> {
    let state_root = install.join(".deploy-desktop");
    if !ensure_real_dir(&state_root, "部署状态目录")? {
        return Ok(Vec::new());
    }
    let root = state_root.join("transactions");
    if !ensure_real_dir(&root, "部署事务目录")? {
        return Ok(Vec::new());
    }
    let directory =
        fs::read_dir(&root).map_err(|error| format!("读取部署事务目录失败：{error}"))?;
    let mut entries = Vec::new();
    for entry in directory {
        let entry = entry.map_err(|error| format!("读取部署事务目录失败：{error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !valid_transaction_name(&name) {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| format!("读取部署事务目录失败：{error}"))?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let Some((state, version)) = transaction_state_and_version(&entry.path()) else {
            continue;
        };
        if !matches!(state.as_str(), "committed" | "recovered") {
            continue;
        }
        let snapshot = entry.path().join(SNAPSHOT_DIR);
        let metadata = match fs::symlink_metadata(&snapshot) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("读取部署事务快照失败：{error}")),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        entries.push(DeploymentBackupEntry {
            name,
            kind: "transaction".to_string(),
            version,
            bytes: tree_bytes(&snapshot)?,
            modified_at: modified_ms(&snapshot),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn transaction_state_and_version(transaction: &Path) -> Option<(String, String)> {
    let path = transaction.join(JOURNAL_FILE);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 64 * 1024 {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let state = value
        .get("state")
        .and_then(|state| state.as_str())?
        .to_string();
    let previous = value
        .get("previousVersion")
        .and_then(|version| version.as_str())
        .unwrap_or("");
    Some((state, sanitize_version(previous)))
}

fn read_update_backup_version(install: &Path, name: &str) -> String {
    let path = install
        .join(BACKUP_DIR)
        .join(BACKUP_META_DIR)
        .join(format!("{name}.json"));
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return String::new();
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4096 {
        return String::new();
    }
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return String::new();
    };
    sanitize_version(
        value
            .get("version")
            .and_then(|version| version.as_str())
            .unwrap_or(""),
    )
}

fn deployed_version(install: &Path) -> String {
    let path = install.join(".deploy-desktop").join("deployed.json");
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return String::new();
    };
    sanitize_version(
        value
            .get("version")
            .and_then(|version| version.as_str())
            .unwrap_or(""),
    )
}

/// 版本号来自用户环境，展示前限制为常见的版本字符并限制长度。
pub(crate) fn sanitize_version(raw: &str) -> String {
    let text = raw.trim();
    if text.is_empty() || text.len() > 128 {
        return String::new();
    }
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
    {
        return String::new();
    }
    text.to_string()
}

/// 更新过程中的备份会写入 `.updating`；超过保留窗口仍存在的标记视为更新进程已中断。
fn backup_is_updating(backup: &Path) -> bool {
    let marker = backup.join(UPDATING_MARKER);
    let Ok(metadata) = fs::metadata(marker) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return false;
    };
    age.as_secs() < UPDATING_STALE_SECS
}

fn sort_entries(entries: &mut [DeploymentBackupEntry]) {
    entries.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| right.name.cmp(&left.name))
    });
}

fn tree_bytes(path: &Path) -> Result<u64, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("读取备份大小失败：{error}")),
    };
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let mut total = 0u64;
    let directory = fs::read_dir(path).map_err(|error| format!("读取备份大小失败：{error}"))?;
    for entry in directory {
        let entry = entry.map_err(|error| format!("读取备份大小失败：{error}"))?;
        total = total.saturating_add(tree_bytes(&entry.path())?);
    }
    Ok(total)
}

fn modified_ms(path: &Path) -> u64 {
    fs::symlink_metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 本机增量更新的备份目录名：`update-<进程号>-<纳秒>-<序号>`。
fn valid_update_backup_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(UPDATE_PREFIX) else {
        return false;
    };
    let parts: Vec<&str> = rest.split('-').collect();
    if parts.len() == 3 && parts.iter().all(|part| is_digits(part)) {
        return true;
    }
    // 远程增量更新的备份目录名：`update-YYYYmmdd-HHMMSS-XXXXXXXX`。
    parts.len() == 3
        && parts[0].len() == 8
        && is_digits(parts[0])
        && parts[1].len() == 6
        && is_digits(parts[1])
        && parts[2].len() == 8
        && parts[2].bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_transaction_name(name: &str) -> bool {
    name.strip_prefix(TRANSACTION_PREFIX).is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn validate_remote_install(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("请先选择部署目录".to_string());
    }
    if text.contains('\0') || text.contains('\n') || text.contains('\r') {
        return Err("部署目录包含无效字符".to_string());
    }
    let path = Path::new(text);
    if !path.is_absolute() || path_guard::has_parent_dir(path) {
        return Err("部署目录必须是目标机上的绝对路径".to_string());
    }
    if text.split('/').any(|part| part == ".") {
        return Err("部署目录不能包含当前目录标记".to_string());
    }
    if !path
        .components()
        .any(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err("部署目录不能是根目录".to_string());
    }
    let normalized = text.trim_end_matches('/');
    if normalized.is_empty() {
        return Err("部署目录不能是根目录".to_string());
    }
    Ok(normalized.to_string())
}

/// 远程脚本已经给出中文原因时直接展示；系统或命令的原始输出只写入日志。
fn remote_error(prefix: &str, stderr: &str) -> String {
    log::warn!(target: "deploy", "{prefix}：{stderr}");
    if stderr
        .chars()
        .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
    {
        return format!("{prefix}：{stderr}");
    }
    format!("{prefix}，请确认部署目录存在且当前账户有权限后重试。")
}

fn remote_scan_script(install: &str) -> String {
    format!(
        r#"set -e
INSTALL={install}
[ -d "$INSTALL" ] || {{ echo '部署目录不存在' >&2; exit 2; }}
ensure_no_symlink_path() {{
  local path=$1
  while [ -n "$path" ] && [ "$path" != "/" ]; do
    [ -L "$path" ] && {{ echo "路径包含符号链接：$path" >&2; exit 3; }}
    path=$(dirname -- "$path")
  done
}}
ensure_no_symlink_path "$INSTALL"
[ ! -L "$INSTALL/backup" ] || {{ echo '备份目录不能是符号链接' >&2; exit 3; }}
[ ! -L "$INSTALL/backup/.meta" ] && {{ [ ! -e "$INSTALL/backup/.meta" ] || [ -d "$INSTALL/backup/.meta" ]; }} || {{ echo '备份版本目录无效' >&2; exit 3; }}
[ ! -L "$INSTALL/.deploy-desktop" ] || {{ echo '部署状态目录不能是符号链接' >&2; exit 3; }}
[ ! -L "$INSTALL/.deploy-desktop/transactions" ] || {{ echo '部署事务目录不能是符号链接' >&2; exit 3; }}
emit_update() {{
  local d=$1
  local name bytes mtime version now updated
  [ -d "$d" ] || return 0
  [ ! -L "$d" ] || return 0
  if [ -f "$d/.updating" ]; then
    now=$(date +%s)
    updated=$(stat -c %Y -- "$d/.updating" 2>/dev/null || stat -f %m -- "$d/.updating" 2>/dev/null || echo 0)
    if [ "$updated" -gt 0 ] && [ $((now - updated)) -lt 21600 ]; then
      return 0
    fi
  fi
  name=${{d##*/}}
  bytes=$(du -sk -- "$d" 2>/dev/null | awk '{{print $1 * 1024}}') || bytes=0
  mtime=$(stat -c %Y -- "$d" 2>/dev/null || stat -f %m -- "$d" 2>/dev/null || echo 0)
  version=
  META="$INSTALL/backup/.meta/$name.json"
  if [ -f "$META" ] && [ ! -L "$META" ]; then
    version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$META" | head -n1)
  fi
  printf 'UPDATE\t%s\t%s\t%s\t%s\n' "$name" "$bytes" "$mtime" "$version"
}}
emit_transaction() {{
  local t=$1
  local state s name bytes mtime version
  [ -d "$t" ] || return 0
  [ ! -L "$t" ] || return 0
  state=
  if [ -f "$t/status.env" ] && [ ! -L "$t/status.env" ]; then
    state=$(grep -m1 '^state=' "$t/status.env" 2>/dev/null | cut -d= -f2- | tr -d '\r' || true)
  fi
  if [ -z "$state" ] && [ -f "$t/journal.json" ] && [ ! -L "$t/journal.json" ]; then
    state=$(sed -n 's/^  "state"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$t/journal.json" | head -n1)
  fi
  case "$state" in
    committed|recovered) ;;
    *) return 0 ;;
  esac
  s=$t/snapshot
  [ -d "$s" ] || return 0
  [ ! -L "$s" ] || return 0
  name=${{t##*/}}
  bytes=$(du -sk -- "$s" 2>/dev/null | awk '{{print $1 * 1024}}') || bytes=0
  mtime=$(stat -c %Y -- "$s" 2>/dev/null || stat -f %m -- "$s" 2>/dev/null || echo 0)
  version=
  if [ -f "$t/journal.json" ] && [ ! -L "$t/journal.json" ]; then
    version=$(sed -n 's/^  "previousVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$t/journal.json" | head -n1)
  fi
  printf 'TXN\t%s\t%s\t%s\t%s\n' "$name" "$bytes" "$mtime" "$version"
}}
for d in "$INSTALL"/backup/update-*; do emit_update "$d"; done
for t in "$INSTALL"/.deploy-desktop/transactions/deploy-*; do emit_transaction "$t"; done
current=
if [ -f "$INSTALL/.deploy-desktop/deployed.json" ]; then
  current=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$INSTALL/.deploy-desktop/deployed.json" | head -n1)
fi
printf 'CURRENT\t%s\n' "$current"
"#,
        install = shell::quote(install)
    )
}

fn remote_cleanup_script(
    install: &str,
    update_targets: &[String],
    transaction_targets: &[String],
) -> String {
    let mut script = format!(
        r#"set -e
INSTALL={install}
[ -d "$INSTALL" ] || {{ echo '部署目录不存在' >&2; exit 2; }}
ensure_no_symlink_path() {{
  local path=$1
  while [ -n "$path" ] && [ "$path" != "/" ]; do
    [ -L "$path" ] && {{ echo "路径包含符号链接：$path" >&2; exit 3; }}
    path=$(dirname -- "$path")
  done
}}
ensure_no_symlink_path "$INSTALL"
[ ! -L "$INSTALL/backup" ] || {{ echo '备份目录不能是符号链接' >&2; exit 3; }}
[ ! -L "$INSTALL/backup/.meta" ] && {{ [ ! -e "$INSTALL/backup/.meta" ] || [ -d "$INSTALL/backup/.meta" ]; }} || {{ echo '备份版本目录无效' >&2; exit 3; }}
[ ! -L "$INSTALL/.deploy-desktop" ] || {{ echo '部署状态目录不能是符号链接' >&2; exit 3; }}
[ ! -L "$INSTALL/.deploy-desktop/transactions" ] || {{ echo '部署事务目录不能是符号链接' >&2; exit 3; }}
LOCK="$INSTALL/.deploy-desktop/deploy.lock"
mkdir -p -- "$INSTALL/.deploy-desktop"
[ ! -L "$LOCK" ] || {{ echo '部署锁不能使用符号链接' >&2; exit 3; }}
command -v flock >/dev/null 2>&1 || {{ echo '目标机缺少 flock，无法确认部署状态' >&2; exit 3; }}
exec 9>>"$LOCK"
flock -n 9 || {{ echo '部署目录正在执行其他任务，请稍后再试' >&2; exit 4; }}
FAILED=0
emit_result() {{
  printf 'RESULT\t%s\t%s\t%s\n' "$1" "$2" "$3"
}}
remove_backup() {{
  local target=$1
  local name=${{target##*/}}
  [ -d "$target" ] || {{ emit_result skipped update "$name"; return 0; }}
  [ ! -L "$target" ] || {{ echo "备份不能是符号链接：$target" >&2; FAILED=1; return 0; }}
  ensure_no_symlink_path "$target"
  [ ! -L "$INSTALL/backup/.meta" ] && {{ [ ! -e "$INSTALL/backup/.meta" ] || [ -d "$INSTALL/backup/.meta" ]; }} || {{ echo '备份版本目录无效' >&2; FAILED=1; return 0; }}
  local meta="$INSTALL/backup/.meta/${{target##*/}}.json"
  if [ -e "$meta" ] || [ -L "$meta" ]; then
    [ ! -L "$meta" ] && [ -f "$meta" ] || {{ echo "备份版本信息不是普通文件：$meta" >&2; FAILED=1; return 0; }}
  fi
  rm -rf -- "$target" || {{ echo "无法清理：$target" >&2; FAILED=1; return 0; }}
  [ ! -e "$target" ] && [ ! -L "$target" ] || {{ echo "清理后仍存在：$target" >&2; FAILED=1; return 0; }}
  rm -f -- "$meta"
  emit_result removed update "$name"
}}
remove_transaction() {{
  local txn=$1
  local name=${{txn##*/}}
  [ -d "$txn" ] || {{ emit_result skipped transaction "$name"; return 0; }}
  [ ! -L "$txn" ] || {{ echo "部署事务不能是符号链接：$txn" >&2; FAILED=1; return 0; }}
  ensure_no_symlink_path "$txn"
  local state=
  if [ -f "$txn/status.env" ] && [ ! -L "$txn/status.env" ]; then
    state=$(grep -m1 '^state=' "$txn/status.env" 2>/dev/null | cut -d= -f2- | tr -d '\r' || true)
  fi
  if [ -z "$state" ] && [ -f "$txn/journal.json" ] && [ ! -L "$txn/journal.json" ]; then
    state=$(sed -n 's/^  "state"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$txn/journal.json" | head -n1)
  fi
  case "$state" in
    committed|recovered) ;;
    *) echo "部署事务尚未完成，已跳过：$txn" >&2; emit_result skipped transaction "$name"; return 0 ;;
  esac
  local snapshot="$txn/snapshot"
  [ -d "$snapshot" ] || {{ emit_result skipped transaction "$name"; return 0; }}
  [ ! -L "$snapshot" ] || {{ echo "事务快照不能是符号链接：$snapshot" >&2; FAILED=1; return 0; }}
  ensure_no_symlink_path "$snapshot"
  rm -rf -- "$snapshot" || {{ echo "无法清理：$snapshot" >&2; FAILED=1; return 0; }}
  [ ! -e "$snapshot" ] && [ ! -L "$snapshot" ] || {{ echo "清理后仍存在：$snapshot" >&2; FAILED=1; return 0; }}
  emit_result removed transaction "$name"
}}
"#,
        install = shell::quote(install)
    );
    for target in update_targets {
        script.push_str(&format!("remove_backup {}\n", shell::quote(target)));
    }
    for target in transaction_targets {
        script.push_str(&format!("remove_transaction {}\n", shell::quote(target)));
    }
    script.push_str("[ \"$FAILED\" = 0 ] || exit 5\necho 'CLEANED=ok'\n");
    script
}

fn parse_remote_entries(
    stdout: &str,
) -> Result<
    (
        Vec<DeploymentBackupEntry>,
        Vec<DeploymentBackupEntry>,
        String,
    ),
    String,
> {
    let mut updates = Vec::new();
    let mut transactions = Vec::new();
    let mut current_version = String::new();
    for line in stdout.lines() {
        let mut parts = line.split('\t');
        let Some(kind) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        if kind == "CURRENT" {
            current_version = sanitize_version(name);
            continue;
        }
        let Some(bytes) = parts.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let modified_at = parts
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            .saturating_mul(1000);
        let version = sanitize_version(parts.next().unwrap_or(""));
        match kind {
            "UPDATE" if valid_update_backup_name(name) => updates.push(DeploymentBackupEntry {
                name: name.to_string(),
                kind: "update".to_string(),
                version,
                bytes,
                modified_at,
            }),
            "TXN" if valid_transaction_name(name) => transactions.push(DeploymentBackupEntry {
                name: name.to_string(),
                kind: "transaction".to_string(),
                version,
                bytes,
                modified_at,
            }),
            _ => {}
        }
    }
    sort_entries(&mut updates);
    sort_entries(&mut transactions);
    Ok((updates, transactions, current_version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_install(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "deploy-backup-cleanup-{label}-{}",
            backup::unique_suffix()
        ));
        fs::create_dir_all(&root).unwrap();
        // macOS 的 /var 是指向 /private/var 的符号链接；远程脚本会逐级拒绝
        // 符号链接，用规范化后的路径才能验证脚本本身的逻辑。
        fs::canonicalize(root).unwrap()
    }

    fn write_journal(install: &Path, id: &str, state: &str) {
        let directory = install.join(deployment_tx::TRANSACTIONS_DIR).join(id);
        fs::create_dir_all(directory.join(SNAPSHOT_DIR).join("volumes")).unwrap();
        fs::write(
            directory
                .join(SNAPSHOT_DIR)
                .join("volumes")
                .join("db.tar.gz"),
            vec![0u8; 128],
        )
        .unwrap();
        fs::write(
            directory.join(JOURNAL_FILE),
            serde_json::to_string_pretty(&serde_json::json!({ "state": state })).unwrap(),
        )
        .unwrap();
    }

    fn write_journal_with_version(
        install: &Path,
        id: &str,
        state: &str,
        version: &str,
        previous_version: &str,
    ) {
        let directory = install.join(deployment_tx::TRANSACTIONS_DIR).join(id);
        fs::create_dir_all(directory.join(SNAPSHOT_DIR)).unwrap();
        fs::write(
            directory.join(JOURNAL_FILE),
            serde_json::to_string_pretty(&serde_json::json!({
                "state": state,
                "version": version,
                "previousVersion": previous_version,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn selection(kind: &str, name: &str) -> DeploymentBackupSelection {
        DeploymentBackupSelection {
            kind: kind.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn update_backup_names_accept_both_layouts() {
        assert!(valid_update_backup_name(
            "update-61633-1794296864083449000-0"
        ));
        assert!(valid_update_backup_name("update-20261118-101500-Ab3dE9xZ"));
        assert!(!valid_update_backup_name("update-20261118-101500"));
        assert!(!valid_update_backup_name("update-backup"));
        assert!(!valid_update_backup_name("parameter_dict_backup_20260928"));
    }

    #[test]
    fn inventory_separates_finished_transactions_from_active_ones() {
        let install = temp_install("inventory");
        fs::create_dir_all(install.join(BACKUP_DIR).join("update-100-200-0")).unwrap();
        fs::write(
            install
                .join(BACKUP_DIR)
                .join("update-100-200-0")
                .join("app.jar"),
            vec![0u8; 64],
        )
        .unwrap();
        fs::write(install.join(BACKUP_DIR).join("notes.txt"), b"keep").unwrap();
        fs::create_dir_all(install.join(".deploy-desktop")).unwrap();
        fs::write(
            install.join(".deploy-desktop").join("deployed.json"),
            r#"{"version":"2.1.0"}"#,
        )
        .unwrap();
        write_journal(&install, "deploy-100-1-0", "committed");
        write_journal(&install, "deploy-200-2-0", "recovered");
        write_journal(&install, "deploy-300-3-0", "failed");
        write_journal(&install, "deploy-400-4-0", "recovering");

        let overview = inspect_local(install.to_str().unwrap()).unwrap();
        assert_eq!(overview.update_backups.len(), 1);
        assert_eq!(overview.update_backups[0].bytes, 64);
        assert_eq!(overview.transaction_snapshots.len(), 2);
        assert_eq!(overview.total_bytes, 64 + 128 * 2);
        assert_eq!(overview.current_version, "2.1.0");

        fs::remove_dir_all(&install).unwrap();
    }

    #[test]
    fn cleanup_deletes_only_selected_backups() {
        let install = temp_install("cleanup");
        for name in ["update-100-200-0", "update-200-300-0", "update-300-400-0"] {
            let directory = install.join(BACKUP_DIR).join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("payload"), vec![0u8; 16]).unwrap();
        }
        write_journal(&install, "deploy-100-1-0", "committed");
        write_journal(&install, "deploy-200-2-0", "failed");

        let result = clean_local(
            install.to_str().unwrap(),
            vec![
                selection("update", "update-200-300-0"),
                selection("transaction", "deploy-100-1-0"),
            ],
        )
        .unwrap();
        assert_eq!(result.removed, 2);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.freed_bytes, 16 + 128);

        let remaining = fs::read_dir(install.join(BACKUP_DIR))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .count();
        assert_eq!(remaining, 2);
        assert!(install
            .join(deployment_tx::TRANSACTIONS_DIR)
            .join("deploy-200-2-0")
            .join(SNAPSHOT_DIR)
            .exists());
        assert!(!install
            .join(deployment_tx::TRANSACTIONS_DIR)
            .join("deploy-100-1-0")
            .join(SNAPSHOT_DIR)
            .exists());

        fs::remove_dir_all(&install).unwrap();
    }

    #[test]
    fn cleanup_skips_unknown_or_unfinished_selections() {
        let install = temp_install("cleanup-skip");
        for name in ["update-100-200-0", "update-200-300-0"] {
            let directory = install.join(BACKUP_DIR).join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("payload"), vec![0u8; 8]).unwrap();
        }
        write_journal(&install, "deploy-100-1-0", "failed");

        let result = clean_local(
            install.to_str().unwrap(),
            vec![
                selection("update", "update-999-999-0"),
                selection("transaction", "deploy-100-1-0"),
            ],
        )
        .unwrap();
        assert_eq!(result.removed, 0);
        assert_eq!(result.skipped, 2);
        assert_eq!(install.join(BACKUP_DIR).read_dir().unwrap().count(), 2);

        fs::remove_dir_all(&install).unwrap();
    }

    #[test]
    fn inventory_reads_version_from_backup_marker() {
        let install = temp_install("versions");
        let backup = install.join(BACKUP_DIR).join("update-100-200-0");
        fs::create_dir_all(&backup).unwrap();
        fs::create_dir_all(install.join(BACKUP_DIR).join(BACKUP_META_DIR)).unwrap();
        fs::write(
            install
                .join(BACKUP_DIR)
                .join(BACKUP_META_DIR)
                .join("update-100-200-0.json"),
            r#"{"version":"1.2.3"}"#,
        )
        .unwrap();
        fs::write(backup.join("payload"), vec![0u8; 8]).unwrap();
        write_journal_with_version(&install, "deploy-100-1-0", "committed", "2.0.0", "1.9.0");

        let overview = inspect_local(install.to_str().unwrap()).unwrap();
        assert_eq!(overview.update_backups[0].version, "1.2.3");
        assert_eq!(overview.transaction_snapshots[0].version, "1.9.0");

        fs::remove_dir_all(&install).unwrap();
    }

    #[test]
    fn in_progress_backup_is_hidden_until_marker_is_stale() {
        let install = temp_install("in-progress");
        let backup = install.join(BACKUP_DIR).join("update-100-200-0");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("payload"), vec![0u8; 8]).unwrap();
        fs::write(backup.join(UPDATING_MARKER), b"").unwrap();

        assert!(inspect_local(install.to_str().unwrap())
            .unwrap()
            .update_backups
            .is_empty());

        let marker = fs::OpenOptions::new()
            .write(true)
            .open(backup.join(UPDATING_MARKER))
            .unwrap();
        marker
            .set_times(fs::FileTimes::new().set_modified(
                SystemTime::now() - std::time::Duration::from_secs(UPDATING_STALE_SECS + 60),
            ))
            .unwrap();
        assert_eq!(
            inspect_local(install.to_str().unwrap())
                .unwrap()
                .update_backups
                .len(),
            1
        );

        fs::remove_dir_all(&install).unwrap();
    }

    #[test]
    fn version_marker_requires_safe_short_text() {
        assert_eq!(sanitize_version("1.2.3"), "1.2.3");
        assert_eq!(sanitize_version("1.2.3-beta+1"), "1.2.3-beta+1");
        assert_eq!(sanitize_version("1.2.3 \"x\""), "");
        assert_eq!(sanitize_version(&"1".repeat(129)), "");
    }

    #[test]
    fn transaction_snapshot_without_previous_version_stays_unrecorded() {
        let install = temp_install("no-previous");
        write_journal_with_version(&install, "deploy-100-1-0", "committed", "2.0.0", "");

        let overview = inspect_local(install.to_str().unwrap()).unwrap();
        assert_eq!(overview.transaction_snapshots.len(), 1);
        assert!(overview.transaction_snapshots[0].version.is_empty());

        fs::remove_dir_all(&install).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_backup_directories_are_ignored() {
        use std::os::unix::fs::symlink;

        let install = temp_install("symlink");
        let outside = temp_install("symlink-outside");
        fs::create_dir_all(install.join(BACKUP_DIR)).unwrap();
        symlink(&outside, install.join(BACKUP_DIR).join("update-100-200-0")).unwrap();

        let overview = inspect_local(install.to_str().unwrap()).unwrap();
        assert!(overview.update_backups.is_empty());
        let result = clean_local(
            install.to_str().unwrap(),
            vec![selection("update", "update-100-200-0")],
        )
        .unwrap();
        assert_eq!(result.removed, 0);
        assert_eq!(result.skipped, 1);
        assert!(outside.exists());

        fs::remove_dir_all(&install).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_backup_root_is_rejected_and_never_touched() {
        use std::os::unix::fs::symlink;

        let install = temp_install("symlink-root");
        let outside = temp_install("symlink-root-outside");
        let external_backup = outside.join("update-100-200-0");
        fs::create_dir_all(&external_backup).unwrap();
        fs::write(external_backup.join("payload"), vec![0u8; 8]).unwrap();
        symlink(&outside, install.join(BACKUP_DIR)).unwrap();

        assert!(inspect_local(install.to_str().unwrap()).is_err());
        assert!(clean_local(
            install.to_str().unwrap(),
            vec![selection("update", "update-100-200-0")]
        )
        .is_err());
        assert!(external_backup.join("payload").exists());

        fs::remove_dir_all(&install).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_backup_meta_is_rejected_and_never_touched() {
        use std::os::unix::fs::symlink;

        let install = temp_install("symlink-meta");
        let outside = temp_install("symlink-meta-outside");
        let backup = install.join(BACKUP_DIR).join("update-100-200-0");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("payload"), vec![0u8; 8]).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let external_meta = outside.join("update-100-200-0.json");
        fs::write(&external_meta, r#"{"version":"9.9.9"}"#).unwrap();
        symlink(&outside, install.join(BACKUP_DIR).join(BACKUP_META_DIR)).unwrap();

        assert!(inspect_local(install.to_str().unwrap()).is_err());
        assert!(clean_local(
            install.to_str().unwrap(),
            vec![selection("update", "update-100-200-0")]
        )
        .is_err());
        assert!(backup.join("payload").exists());
        assert!(external_meta.exists());

        fs::remove_dir_all(&install).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_version_file_is_ignored_and_removed_with_the_backup() {
        use std::os::unix::fs::symlink;

        let install = temp_install("symlink-meta-file");
        let outside = temp_install("symlink-meta-file-outside");
        let backup = install.join(BACKUP_DIR).join("update-100-200-0");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("payload"), vec![0u8; 8]).unwrap();
        let meta_dir = install.join(BACKUP_DIR).join(BACKUP_META_DIR);
        fs::create_dir_all(&meta_dir).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let external = outside.join("version.json");
        fs::write(&external, r#"{"version":"9.9.9"}"#).unwrap();
        symlink(&external, meta_dir.join("update-100-200-0.json")).unwrap();

        let overview = inspect_local(install.to_str().unwrap()).unwrap();
        assert_eq!(overview.update_backups.len(), 1);
        assert!(overview.update_backups[0].version.is_empty());

        let result = clean_local(
            install.to_str().unwrap(),
            vec![selection("update", "update-100-200-0")],
        )
        .unwrap();
        assert_eq!(result.removed, 1);
        assert!(!backup.exists());
        assert!(!meta_dir.join("update-100-200-0.json").exists());
        assert!(external.exists());

        fs::remove_dir_all(&install).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_transaction_journal_is_never_treated_as_finished() {
        use std::os::unix::fs::symlink;

        let install = temp_install("symlink-journal");
        let outside = temp_install("symlink-journal-outside");
        fs::create_dir_all(&outside).unwrap();
        let external = outside.join("journal.json");
        fs::write(
            &external,
            serde_json::to_string_pretty(&serde_json::json!({ "state": "committed" })).unwrap(),
        )
        .unwrap();
        let transaction = install
            .join(deployment_tx::TRANSACTIONS_DIR)
            .join("deploy-100-1-0");
        fs::create_dir_all(transaction.join(SNAPSHOT_DIR)).unwrap();
        fs::write(transaction.join(SNAPSHOT_DIR).join("payload"), vec![0u8; 8]).unwrap();
        symlink(&external, transaction.join(JOURNAL_FILE)).unwrap();

        let overview = inspect_local(install.to_str().unwrap()).unwrap();
        assert!(overview.transaction_snapshots.is_empty());

        let result = clean_local(
            install.to_str().unwrap(),
            vec![selection("transaction", "deploy-100-1-0")],
        )
        .unwrap();
        assert_eq!(result.removed, 0);
        assert_eq!(result.skipped, 1);
        assert!(transaction.join(SNAPSHOT_DIR).join("payload").exists());
        assert!(external.exists());

        fs::remove_dir_all(&install).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remote_scan_reads_final_state_from_status_env() {
        use std::os::unix::fs::symlink;

        let install = temp_install("remote-scan");
        fs::create_dir_all(
            install
                .join(BACKUP_DIR)
                .join("update-20261118-101500-Ab3dE9xZ"),
        )
        .unwrap();
        fs::create_dir_all(install.join(".deploy-desktop")).unwrap();
        fs::write(
            install.join(".deploy-desktop").join("deployed.json"),
            r#"{"version":"2.0.0"}"#,
        )
        .unwrap();
        fs::write(
            install
                .join(BACKUP_DIR)
                .join("update-20261118-101500-Ab3dE9xZ")
                .join("payload"),
            vec![0u8; 16],
        )
        .unwrap();
        fs::create_dir_all(install.join(BACKUP_DIR).join(BACKUP_META_DIR)).unwrap();
        fs::write(
            install
                .join(BACKUP_DIR)
                .join(BACKUP_META_DIR)
                .join("update-20261118-101500-Ab3dE9xZ.json"),
            r#"{"version":"1.2.3"}"#,
        )
        .unwrap();
        let in_progress = install
            .join(BACKUP_DIR)
            .join("update-20261118-111111-ZZZZZZZZ");
        fs::create_dir_all(&in_progress).unwrap();
        fs::write(in_progress.join("payload"), vec![0u8; 8]).unwrap();
        fs::write(in_progress.join(UPDATING_MARKER), b"").unwrap();
        write_journal_with_version(&install, "deploy-100-1-0", "pending", "2.0.0", "1.8.0");
        fs::write(
            install
                .join(deployment_tx::TRANSACTIONS_DIR)
                .join("deploy-100-1-0")
                .join("status.env"),
            "phase=recovered\nstate=recovered\n",
        )
        .unwrap();
        write_journal(&install, "deploy-200-2-0", "pending");
        fs::write(
            install
                .join(deployment_tx::TRANSACTIONS_DIR)
                .join("deploy-200-2-0")
                .join("status.env"),
            "phase=recovering\nstate=recovering\n",
        )
        .unwrap();
        write_journal(&install, "deploy-300-3-0", "pending");
        let fake_status = install.join("fake-status.env");
        fs::write(&fake_status, "state=committed\n").unwrap();
        symlink(
            &fake_status,
            install
                .join(deployment_tx::TRANSACTIONS_DIR)
                .join("deploy-300-3-0")
                .join("status.env"),
        )
        .unwrap();

        let script = remote_scan_script(install.to_str().unwrap());
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(&script)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (updates, transactions, current_version) =
            parse_remote_entries(&String::from_utf8_lossy(&output.stdout)).unwrap();
        assert_eq!(updates.len(), 1);
        assert!(updates[0].bytes > 0);
        assert_eq!(updates[0].version, "1.2.3");
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].name, "deploy-100-1-0");
        assert_eq!(transactions[0].version, "1.8.0");
        assert_eq!(current_version, "2.0.0");

        fs::remove_dir_all(&install).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remote_scan_rejects_symlinked_install_root() {
        use std::os::unix::fs::symlink;

        let parent = temp_install("remote-link-parent");
        let real = parent.join("real");
        fs::create_dir_all(&real).unwrap();
        let link = parent.join("link");
        symlink(&real, &link).unwrap();

        let script = remote_scan_script(link.to_str().unwrap());
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(&script)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("符号链接"));

        fs::remove_dir_all(&parent).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remote_scan_rejects_symlinked_meta_and_ignores_symlinked_version_file() {
        use std::os::unix::fs::symlink;

        let install = temp_install("remote-meta-link");
        let outside = temp_install("remote-meta-link-outside");
        fs::create_dir_all(&outside).unwrap();
        let backup = install.join(BACKUP_DIR).join("update-20261118-101500-Ab3dE9xZ");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("payload"), vec![0u8; 8]).unwrap();
        symlink(&outside, install.join(BACKUP_DIR).join(BACKUP_META_DIR)).unwrap();

        let script = remote_scan_script(install.to_str().unwrap());
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(&script)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("备份版本目录"));

        fs::remove_file(install.join(BACKUP_DIR).join(BACKUP_META_DIR)).unwrap();
        symlink(
            outside.join("missing"),
            install.join(BACKUP_DIR).join(BACKUP_META_DIR),
        )
        .unwrap();
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(&script)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("备份版本目录"));

        fs::remove_file(install.join(BACKUP_DIR).join(BACKUP_META_DIR)).unwrap();
        let meta_dir = install.join(BACKUP_DIR).join(BACKUP_META_DIR);
        fs::create_dir_all(&meta_dir).unwrap();
        let external = outside.join("version.json");
        fs::write(&external, r#"{"version":"9.9.9"}"#).unwrap();
        symlink(
            &external,
            meta_dir.join("update-20261118-101500-Ab3dE9xZ.json"),
        )
        .unwrap();

        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(&script)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (updates, _, _) =
            parse_remote_entries(&String::from_utf8_lossy(&output.stdout)).unwrap();
        assert_eq!(updates.len(), 1);
        assert!(updates[0].version.is_empty());

        fs::remove_dir_all(&install).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remote_cleanup_passes_each_target_instead_of_the_root() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::fs::symlink;

        let install = temp_install("remote-cleanup");
        let target = install.join(BACKUP_DIR).join("update-100-200-0");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("payload"), vec![0u8; 8]).unwrap();
        write_journal(&install, "deploy-100-1-0", "committed");
        write_journal(&install, "deploy-200-2-0", "failed");
        write_journal(&install, "deploy-300-3-0", "pending");
        let fake_status = install.join("fake-status.env");
        fs::write(&fake_status, "state=committed\n").unwrap();
        let committed = install
            .join(deployment_tx::TRANSACTIONS_DIR)
            .join("deploy-100-1-0");
        let failed = install
            .join(deployment_tx::TRANSACTIONS_DIR)
            .join("deploy-200-2-0");
        let linked = install
            .join(deployment_tx::TRANSACTIONS_DIR)
            .join("deploy-300-3-0");
        symlink(&fake_status, linked.join("status.env")).unwrap();

        let bin = install.join("fake-bin");
        fs::create_dir_all(&bin).unwrap();
        let log = install.join("rm.log");
        let fake_rm = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\nfor last in \"$@\"; do :; done\n/bin/rm -rf -- \"$last\"\n",
            shell::quote(log.to_str().unwrap())
        );
        for (name, body) in [
            ("flock", "#!/bin/sh\nexit 0\n".to_string()),
            ("rm", fake_rm),
        ] {
            let path = bin.join(name);
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let script = remote_cleanup_script(
            install.to_str().unwrap(),
            &[target.to_string_lossy().to_string()],
            &[
                committed.to_string_lossy().to_string(),
                failed.to_string_lossy().to_string(),
                linked.to_string_lossy().to_string(),
            ],
        );
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(&script)
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("RESULT\tremoved\tupdate\tupdate-100-200-0"));
        assert!(stdout.contains("RESULT\tremoved\ttransaction\tdeploy-100-1-0"));
        assert!(stdout.contains("RESULT\tskipped\ttransaction\tdeploy-200-2-0"));
        assert!(stdout.contains("RESULT\tskipped\ttransaction\tdeploy-300-3-0"));
        let recorded = fs::read_to_string(&log).unwrap();
        assert!(recorded
            .lines()
            .any(|line| line == target.to_string_lossy()));
        assert!(recorded
            .lines()
            .any(|line| line == committed.join(SNAPSHOT_DIR).to_string_lossy()));
        assert!(!recorded
            .lines()
            .any(|line| line == failed.join(SNAPSHOT_DIR).to_string_lossy()));
        assert!(!recorded
            .lines()
            .any(|line| line == linked.join(SNAPSHOT_DIR).to_string_lossy()));
        assert!(!recorded.lines().any(|line| line == "/"));
        assert!(!target.exists());
        assert!(!committed.join(SNAPSHOT_DIR).exists());
        assert!(failed.join(SNAPSHOT_DIR).exists());
        assert!(linked.join(SNAPSHOT_DIR).exists());

        fs::remove_dir_all(&install).unwrap();
    }
}
