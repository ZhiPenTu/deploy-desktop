//! 本机磁盘可用空间。
//!
//! Unix 直接读取 `statvfs`，Windows 通过系统自带的 PowerShell 查询卷剩余空间；
//! 两端都不依赖 `df` 等在 Windows 上不存在的命令。

use std::path::{Path, PathBuf};

#[cfg(unix)]
pub fn available_bytes(path: &Path) -> Result<u64, String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let anchor = anchor_path(path);
    let raw = CString::new(anchor.as_os_str().as_bytes())
        .map_err(|_| "无法检查部署分区空间".to_string())?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let code = unsafe { libc::statvfs(raw.as_ptr(), &mut stat) };
    if code != 0 {
        return Err("无法检查部署分区空间".to_string());
    }
    let block_size = if stat.f_frsize > 0 {
        stat.f_frsize as u64
    } else {
        stat.f_bsize as u64
    };
    Ok(stat.f_bavail as u64 * block_size)
}

#[cfg(windows)]
pub fn available_bytes(path: &Path) -> Result<u64, String> {
    let anchor = anchor_path(path);
    let mut command = std::process::Command::new("powershell");
    crate::core::process::hide_console(&mut command);
    let output = command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-PSDrive -PSProvider FileSystem | Select-Object Root,Free | ConvertTo-Json -Compress",
        ])
        .output()
        .map_err(|_| "无法检查部署分区空间".to_string())?;
    if !output.status.success() {
        return Err("无法检查部署分区空间".to_string());
    }
    let drives: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "无法检查部署分区空间".to_string())?;
    let entries: Vec<&serde_json::Value> = match drives.as_array() {
        Some(entries) => entries.iter().collect(),
        None => vec![&drives],
    };
    let root = anchor
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_lowercase();
    entries
        .iter()
        .find_map(|drive| {
            let drive_root = drive.get("Root")?.as_str()?.to_lowercase();
            root.starts_with(&drive_root)
                .then(|| drive.get("Free").and_then(serde_json::Value::as_u64))
                .flatten()
        })
        .ok_or_else(|| "无法检查部署分区空间".to_string())
}

#[cfg(not(any(unix, windows)))]
pub fn available_bytes(_path: &Path) -> Result<u64, String> {
    Err("当前平台不支持检查磁盘空间".to_string())
}

fn anchor_path(path: &Path) -> PathBuf {
    if path.exists() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn reports_available_space_for_existing_directory() {
        assert!(available_bytes(Path::new("/")).unwrap() > 0);
    }
}
