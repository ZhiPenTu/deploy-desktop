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
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

const BACKUP_DIR: &str = "backup";
const UPDATE_PREFIX: &str = "update-";
const TRANSACTION_PREFIX: &str = "deploy-";
const SNAPSHOT_DIR: &str = "snapshot";
const JOURNAL_FILE: &str = "journal.json";
const REMOTE_CLEANUP_TIMEOUT_SECS: u64 = 600;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentBackupEntry {
    pub name: String,
    pub kind: String,
    pub bytes: u64,
    pub modified_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentBackupOverview {
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
    pub kept: usize,
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
) -> Result<DeploymentBackupCleanup, String> {
    crate::commands::blocking::run(move || clean_local(&install_dir)).await
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
) -> Result<DeploymentBackupCleanup, String> {
    crate::commands::blocking::run(move || clean_remote(&host, port, &user, key_path, &install_dir))
        .await
}

fn inspect_local(install_dir: &str) -> Result<DeploymentBackupOverview, String> {
    let install = version_deploy::resolve_install_root(install_dir, true)?;
    let update_backups = local_update_backups(&install)?;
    let transaction_snapshots = local_transaction_snapshots(&install)?;
    Ok(overview(update_backups, transaction_snapshots))
}

fn clean_local(install_dir: &str) -> Result<DeploymentBackupCleanup, String> {
    let install = version_deploy::resolve_install_root(install_dir, true)?;
    let _lock = deployment_tx::DeploymentLock::acquire(&install)?;
    let update_backups = local_update_backups(&install)?;
    let transaction_snapshots = local_transaction_snapshots(&install)?;

    // 增量更新在备份目录中复制文件时不会全程持有部署锁；始终保留最新一份
    // 更新备份，避免把正在写入或刚生成的回滚副本当作历史数据删除。
    let keep = update_backups.first().map(|entry| entry.name.clone());
    let mut result = DeploymentBackupCleanup {
        removed: 0,
        freed_bytes: 0,
        kept: 0,
    };
    for entry in &update_backups {
        if keep.as_deref() == Some(entry.name.as_str()) {
            result.kept += 1;
            continue;
        }
        let target = install.join(BACKUP_DIR).join(&entry.name);
        remove_checked(&install.join(BACKUP_DIR), &target, &entry.name, &mut result).map_err(
            |error| {
                format!(
                    "{error}；已清理 {} 项，释放 {} 字节",
                    result.removed, result.freed_bytes
                )
            },
        )?;
    }
    for entry in &transaction_snapshots {
        let target = install
            .join(deployment_tx::TRANSACTIONS_DIR)
            .join(&entry.name)
            .join(SNAPSHOT_DIR);
        remove_checked(
            &install.join(deployment_tx::TRANSACTIONS_DIR),
            &target,
            &entry.name,
            &mut result,
        )
        .map_err(|error| {
            format!(
                "{error}；已清理 {} 项，释放 {} 字节",
                result.removed, result.freed_bytes
            )
        })?;
    }
    Ok(result)
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
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
    let (update_backups, transaction_snapshots) =
        parse_remote_entries(&String::from_utf8_lossy(&output.stdout))?;
    Ok(overview(update_backups, transaction_snapshots))
}

fn clean_remote(
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<String>,
    install_dir: &str,
) -> Result<DeploymentBackupCleanup, String> {
    let install = validate_remote_install(install_dir)?;
    let key = remote_ssh::resolve_existing_key(key_path, host)?;
    let scan = remote_scan_script(&install);
    let output = remote_ssh::run_ssh_script(host, port, user, key.as_deref(), &scan)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(remote_error("读取目标机部署备份失败", &stderr));
    }
    let (update_backups, transaction_snapshots) =
        parse_remote_entries(&String::from_utf8_lossy(&output.stdout))?;
    let keep = update_backups.first().map(|entry| entry.name.clone());

    let mut targets = Vec::new();
    let mut removed = 0usize;
    let mut freed_bytes = 0u64;
    let mut kept = 0usize;
    for entry in &update_backups {
        if keep.as_deref() == Some(entry.name.as_str()) {
            kept += 1;
            continue;
        }
        targets.push(format!("{install}/{BACKUP_DIR}/{}", entry.name));
        removed += 1;
        freed_bytes = freed_bytes.saturating_add(entry.bytes);
    }
    for entry in &transaction_snapshots {
        targets.push(format!(
            "{install}/{}/{}/{SNAPSHOT_DIR}",
            deployment_tx::TRANSACTIONS_DIR,
            entry.name
        ));
        removed += 1;
        freed_bytes = freed_bytes.saturating_add(entry.bytes);
    }
    if targets.is_empty() {
        return Ok(DeploymentBackupCleanup {
            removed,
            freed_bytes,
            kept,
        });
    }

    let script = remote_cleanup_script(&install, &targets);
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
    Ok(DeploymentBackupCleanup {
        removed,
        freed_bytes,
        kept,
    })
}

fn overview(
    update_backups: Vec<DeploymentBackupEntry>,
    transaction_snapshots: Vec<DeploymentBackupEntry>,
) -> DeploymentBackupOverview {
    let update_bytes = update_backups.iter().map(|entry| entry.bytes).sum();
    let transaction_bytes = transaction_snapshots.iter().map(|entry| entry.bytes).sum();
    DeploymentBackupOverview {
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
        entries.push(DeploymentBackupEntry {
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
        if !transaction_is_finished(&entry.path()) {
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
            bytes: tree_bytes(&snapshot)?,
            modified_at: modified_ms(&snapshot),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn transaction_is_finished(transaction: &Path) -> bool {
    let Ok(text) = fs::read_to_string(transaction.join(JOURNAL_FILE)) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    matches!(
        value.get("state").and_then(|state| state.as_str()),
        Some("committed") | Some("recovered")
    )
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
[ ! -L "$INSTALL/.deploy-desktop" ] || {{ echo '部署状态目录不能是符号链接' >&2; exit 3; }}
[ ! -L "$INSTALL/.deploy-desktop/transactions" ] || {{ echo '部署事务目录不能是符号链接' >&2; exit 3; }}
emit_update() {{
  local d=$1
  local name bytes mtime
  [ -d "$d" ] || return 0
  [ ! -L "$d" ] || return 0
  name=${{d##*/}}
  bytes=$(du -sk -- "$d" 2>/dev/null | awk '{{print $1 * 1024}}') || bytes=0
  mtime=$(stat -c %Y -- "$d" 2>/dev/null || stat -f %m -- "$d" 2>/dev/null || echo 0)
  printf 'UPDATE\t%s\t%s\t%s\n' "$name" "$bytes" "$mtime"
}}
emit_transaction() {{
  local t=$1
  local state s name bytes mtime
  [ -d "$t" ] || return 0
  [ ! -L "$t" ] || return 0
  state=
  if [ -f "$t/status.env" ]; then
    state=$(grep -m1 '^state=' "$t/status.env" 2>/dev/null | cut -d= -f2- | tr -d '\r' || true)
  fi
  if [ -z "$state" ] && [ -f "$t/journal.json" ]; then
    if grep -Eq '"state"[[:space:]]*:[[:space:]]*"committed"' "$t/journal.json"; then
      state=committed
    elif grep -Eq '"state"[[:space:]]*:[[:space:]]*"recovered"' "$t/journal.json"; then
      state=recovered
    fi
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
  printf 'TXN\t%s\t%s\t%s\n' "$name" "$bytes" "$mtime"
}}
for d in "$INSTALL"/backup/update-*; do emit_update "$d"; done
for t in "$INSTALL"/.deploy-desktop/transactions/deploy-*; do emit_transaction "$t"; done
"#,
        install = shell::quote(install)
    )
}

fn remote_cleanup_script(install: &str, targets: &[String]) -> String {
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
[ ! -L "$INSTALL/.deploy-desktop" ] || {{ echo '部署状态目录不能是符号链接' >&2; exit 3; }}
[ ! -L "$INSTALL/.deploy-desktop/transactions" ] || {{ echo '部署事务目录不能是符号链接' >&2; exit 3; }}
LOCK="$INSTALL/.deploy-desktop/deploy.lock"
mkdir -p -- "$INSTALL/.deploy-desktop"
[ ! -L "$LOCK" ] || {{ echo '部署锁不能使用符号链接' >&2; exit 3; }}
command -v flock >/dev/null 2>&1 || {{ echo '目标机缺少 flock，无法确认部署状态' >&2; exit 3; }}
exec 9>>"$LOCK"
flock -n 9 || {{ echo '部署目录正在执行其他任务，请稍后再试' >&2; exit 4; }}
FAILED=0
remove_backup() {{
  local target=$1
  [ -d "$target" ] || return 0
  [ ! -L "$target" ] || {{ echo "备份不能是符号链接：$target" >&2; FAILED=1; return 0; }}
  ensure_no_symlink_path "$target"
  rm -rf -- "$target" || {{ echo "无法清理：$target" >&2; FAILED=1; }}
}}
"#,
        install = shell::quote(install)
    );
    for target in targets {
        script.push_str(&format!("remove_backup {}\n", shell::quote(target)));
    }
    script.push_str("[ \"$FAILED\" = 0 ] || exit 5\necho 'CLEANED=ok'\n");
    script
}

fn parse_remote_entries(
    stdout: &str,
) -> Result<(Vec<DeploymentBackupEntry>, Vec<DeploymentBackupEntry>), String> {
    let mut updates = Vec::new();
    let mut transactions = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.split('\t');
        let Some(kind) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        let Some(bytes) = parts.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let modified_at = parts
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            .saturating_mul(1000);
        match kind {
            "UPDATE" if valid_update_backup_name(name) => updates.push(DeploymentBackupEntry {
                name: name.to_string(),
                kind: "update".to_string(),
                bytes,
                modified_at,
            }),
            "TXN" if valid_transaction_name(name) => transactions.push(DeploymentBackupEntry {
                name: name.to_string(),
                kind: "transaction".to_string(),
                bytes,
                modified_at,
            }),
            _ => {}
        }
    }
    sort_entries(&mut updates);
    sort_entries(&mut transactions);
    Ok((updates, transactions))
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
            format!(r#"{{"state":"{state}"}}"#),
        )
        .unwrap();
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
        write_journal(&install, "deploy-100-1-0", "committed");
        write_journal(&install, "deploy-200-2-0", "recovered");
        write_journal(&install, "deploy-300-3-0", "failed");
        write_journal(&install, "deploy-400-4-0", "recovering");

        let overview = inspect_local(install.to_str().unwrap()).unwrap();
        assert_eq!(overview.update_backups.len(), 1);
        assert_eq!(overview.update_backups[0].bytes, 64);
        assert_eq!(overview.transaction_snapshots.len(), 2);
        assert_eq!(overview.total_bytes, 64 + 128 * 2);

        fs::remove_dir_all(&install).unwrap();
    }

    #[test]
    fn cleanup_removes_update_backups_and_finished_snapshots_only() {
        let install = temp_install("cleanup");
        for name in ["update-100-200-0", "update-200-300-0", "update-300-400-0"] {
            let directory = install.join(BACKUP_DIR).join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("payload"), vec![0u8; 16]).unwrap();
        }
        write_journal(&install, "deploy-100-1-0", "committed");
        write_journal(&install, "deploy-200-2-0", "failed");

        let result = clean_local(install.to_str().unwrap()).unwrap();
        assert_eq!(result.removed, 3);
        assert_eq!(result.kept, 1);
        assert_eq!(result.freed_bytes, 2 * 16 + 128);

        let remaining = fs::read_dir(install.join(BACKUP_DIR))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .count();
        assert_eq!(remaining, 1);
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
    fn cleanup_always_keeps_the_newest_update_backup() {
        let install = temp_install("cleanup-all");
        for name in ["update-100-200-0", "update-200-300-0"] {
            let directory = install.join(BACKUP_DIR).join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("payload"), vec![0u8; 8]).unwrap();
        }

        let result = clean_local(install.to_str().unwrap()).unwrap();
        assert_eq!(result.removed, 1);
        assert_eq!(result.kept, 1);
        assert_eq!(install.join(BACKUP_DIR).read_dir().unwrap().count(), 1);

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
        let result = clean_local(install.to_str().unwrap()).unwrap();
        assert_eq!(result.removed, 0);
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
        assert!(clean_local(install.to_str().unwrap()).is_err());
        assert!(external_backup.join("payload").exists());

        fs::remove_dir_all(&install).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remote_scan_reads_final_state_from_status_env() {
        let install = temp_install("remote-scan");
        fs::create_dir_all(
            install
                .join(BACKUP_DIR)
                .join("update-20261118-101500-Ab3dE9xZ"),
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
        write_journal(&install, "deploy-100-1-0", "pending");
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
        let (updates, transactions) =
            parse_remote_entries(&String::from_utf8_lossy(&output.stdout)).unwrap();
        assert_eq!(updates.len(), 1);
        assert!(updates[0].bytes > 0);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].name, "deploy-100-1-0");

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
    fn remote_cleanup_passes_each_target_instead_of_the_root() {
        use std::os::unix::fs::PermissionsExt;

        let install = temp_install("remote-cleanup");
        let target = install.join(BACKUP_DIR).join("update-100-200-0");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("payload"), vec![0u8; 8]).unwrap();

        let bin = install.join("fake-bin");
        fs::create_dir_all(&bin).unwrap();
        let log = install.join("rm.log");
        let fake_rm = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\nexit 0\n",
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
        let recorded = fs::read_to_string(&log).unwrap();
        assert!(recorded
            .lines()
            .any(|line| line == target.to_string_lossy()));
        assert!(!recorded.lines().any(|line| line == "/"));
        assert!(target.join("payload").exists());

        fs::remove_dir_all(&install).unwrap();
    }
}
