//! 本机文件的安全覆盖写入。
//!
//! 直接 `fs::write` 会先截断目标文件，进程在写入途中退出就会留下半截 JSON；
//! 这里改为「唯一临时文件 → fsync → rename」，rename 在 Windows 与 Unix 上都会
//! 整体替换目标文件，读取方只会看到旧内容或新内容。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 目标文件的权限策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePermission {
    /// 仅当前用户可读写（Unix 0600），用于凭据、密码与主机连接配置。
    OwnerOnly,
    /// 跟随系统默认权限，用于不含机密内容的配置与缓存。
    Default,
}

const MAX_TEMP_ATTEMPTS: usize = 16;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn write_text_atomic(
    path: &Path,
    text: &str,
    permission: FilePermission,
) -> std::io::Result<()> {
    write_atomic(path, text.as_bytes(), permission)
}

/// 原子覆盖写入。失败时清理临时文件，目标文件保持上一次的完整内容。
pub fn write_atomic(path: &Path, bytes: &[u8], permission: FilePermission) -> std::io::Result<()> {
    let directory = parent_directory(path);
    let (temporary, mut file) = create_temporary(&directory, path, permission)?;

    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(file);

    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    sync_directory(&directory);
    Ok(())
}

fn parent_directory(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// 进程号、时间戳与自增序号共同保证同一目录下的并发写入不会争用同一个临时文件。
fn temporary_name(path: &Path) -> String {
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("config");
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!(".{stem}.{}.{nanos}.{sequence}.tmp", std::process::id())
}

fn create_temporary(
    directory: &Path,
    path: &Path,
    permission: FilePermission,
) -> std::io::Result<(PathBuf, File)> {
    let mut last_conflict = None;
    for _ in 0..MAX_TEMP_ATTEMPTS {
        let candidate = directory.join(temporary_name(path));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            if permission == FilePermission::OwnerOnly {
                options.mode(0o600);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = permission;
        }
        match options.open(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_conflict = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_conflict.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "临时文件名重复")
    }))
}

/// 让目录项也落盘，避免 rename 在断电后丢失。部分文件系统不支持对目录 fsync，忽略即可。
fn sync_directory(directory: &Path) {
    #[cfg(unix)]
    {
        if let Ok(handle) = File::open(directory) {
            let _ = handle.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "deploy-desktop-core-atomic-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn replaces_existing_file_with_new_content() {
        let dir = temp_dir("replace");
        let path = dir.join("config.json");

        write_text_atomic(&path, "{\"v\":1}", FilePermission::Default).unwrap();
        write_text_atomic(&path, "{\"v\":22}", FilePermission::Default).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"v\":22}");
        assert_eq!(entries(&dir), vec!["config.json".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_writes_leave_one_complete_document() {
        let dir = temp_dir("concurrent");
        let path = dir.join("config.json");
        let mut handles = Vec::new();

        for worker in 0..8 {
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                let payload = format!(
                    "{{\"worker\":{worker},\"padding\":\"{}\"}}",
                    "x".repeat(worker * 64)
                );
                for _ in 0..20 {
                    write_text_atomic(&path, &payload, FilePermission::OwnerOnly).unwrap();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let content = fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(value.get("worker").and_then(|item| item.as_u64()).is_some());
        assert_eq!(entries(&dir), vec!["config.json".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_error_when_directory_is_missing() {
        let dir = temp_dir("missing");
        let path = dir.join("absent").join("config.json");

        assert!(write_text_atomic(&path, "{}", FilePermission::Default).is_err());
        assert!(entries(&dir).is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_files_are_not_group_or_world_readable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("perm");
        let path = dir.join("secrets.json");
        write_text_atomic(&path, "{}", FilePermission::OwnerOnly).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(&dir);
    }
}
