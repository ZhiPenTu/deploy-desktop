//! 应用配置目录的唯一解析入口。

use std::fs;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

/// 本机配置、凭据与缓存的根目录；不存在时自动创建。
pub fn app_config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("读取应用配置目录失败：{error}"))?;
    ensure_dir(&dir)?;
    Ok(dir)
}

pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|error| format!("创建应用配置目录失败：{error}"))
}
