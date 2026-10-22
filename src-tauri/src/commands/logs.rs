//! 运行日志的读取与维护命令。
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

use tauri::AppHandle;

use crate::logging::{self, BACKUP_FILE, LOG_FILE, MAX_TOTAL_BYTES};

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogCounts {
    pub info: usize,
    pub warn: usize,
    pub error: usize,
    pub debug: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogEntry {
    pub time: String,
    pub date: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogSnapshot {
    pub dir: String,
    pub entries: Vec<RuntimeLogEntry>,
    pub total_bytes: u64,
    pub max_bytes: u64,
    pub counts: RuntimeLogCounts,
    pub matched: usize,
    pub truncated: bool,
    pub level: String,
}

fn log_files(dir: &PathBuf) -> Vec<PathBuf> {
    // 先读旧文件，保证同一时刻内较早的记录排在前面。
    [BACKUP_FILE, LOG_FILE]
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

fn parse_line(line: &str) -> RuntimeLogEntry {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let mut parts = trimmed.splitn(4, ' ');
    let date = parts.next().unwrap_or_default();
    let clock = parts.next().unwrap_or_default();
    let level = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default();
    if date.is_empty() || clock.is_empty() || !matches!(level, "INFO" | "WARN" | "ERROR" | "DEBUG" | "TRACE") {
        return RuntimeLogEntry {
            time: String::new(),
            date: String::new(),
            level: "INFO".to_string(),
            target: "app".to_string(),
            message: trimmed.to_string(),
        };
    }
    let (target, message) = rest.split_once(" - ").unwrap_or(("app", rest));
    RuntimeLogEntry {
        time: format!("{date} {clock}"),
        date: date.to_string(),
        level: level.to_string(),
        target: target.trim().to_string(),
        message: message.trim().to_string(),
    }
}

fn matches_level(level: &str, filter: &str) -> bool {
    match filter.trim().to_ascii_lowercase().as_str() {
        "info" => level == "INFO",
        "warn" | "warning" => level == "WARN",
        "error" => level == "ERROR",
        "debug" => matches!(level, "DEBUG" | "TRACE"),
        _ => true,
    }
}

fn matches_keyword(entry: &RuntimeLogEntry, keyword: &str) -> bool {
    if keyword.is_empty() {
        return true;
    }
    let needle = keyword.to_lowercase();
    entry.message.to_lowercase().contains(&needle) || entry.target.to_lowercase().contains(&needle)
}

#[tauri::command]
pub async fn read_runtime_logs(
    app: AppHandle,
    level: Option<String>,
    limit: Option<usize>,
    keyword: Option<String>,
) -> Result<RuntimeLogSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = logging::log_dir_or_app(&app)?;
        let filter = level.unwrap_or_default();
        let keyword = keyword.unwrap_or_default().trim().to_string();
        let limit = limit.unwrap_or(500).clamp(1, 2000);
        let mut entries = Vec::new();
        let mut counts = RuntimeLogCounts::default();
        for path in log_files(&dir) {
            let content = fs::read_to_string(&path)
                .map_err(|error| format!("读取运行日志失败：{error}"))?;
            for line in content.lines().filter(|line| !line.trim().is_empty()) {
                let entry = parse_line(line);
                match entry.level.as_str() {
                    "ERROR" => counts.error += 1,
                    "WARN" => counts.warn += 1,
                    "DEBUG" | "TRACE" => counts.debug += 1,
                    _ => counts.info += 1,
                }
                if matches_level(&entry.level, &filter) && matches_keyword(&entry, &keyword) {
                    entries.push(entry);
                }
            }
        }
        let matched = entries.len();
        // 界面按最新在前展示。
        entries.reverse();
        entries.truncate(limit);
        Ok(RuntimeLogSnapshot {
            dir: dir.to_string_lossy().to_string(),
            truncated: matched > entries.len(),
            entries,
            total_bytes: logging::total_bytes(&dir),
            max_bytes: MAX_TOTAL_BYTES,
            counts,
            matched,
            level: logging::current_level(),
        })
    })
    .await
    .map_err(|error| format!("读取运行日志失败：{error}"))?
}

#[tauri::command]
pub fn clear_runtime_logs() -> Result<(), String> {
    logging::clear()?;
    log::info!(target: "app", "运行日志已清空");
    Ok(())
}

#[tauri::command]
pub fn set_runtime_log_level(level: String) -> Result<String, String> {
    let next = logging::set_level(&level)?;
    if next == "debug" {
        log::info!(target: "app", "已开启详细调试日志，之后的每一次操作都会记录");
    } else {
        log::info!(target: "app", "已关闭详细调试日志");
    }
    Ok(next)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLogSettings {
    pub dir: String,
    pub default_dir: String,
    pub is_custom: bool,
}

fn settings_snapshot(app: &AppHandle) -> Result<RuntimeLogSettings, String> {
    let dir = logging::log_dir_or_app(app)?;
    let default_dir = logging::default_dir(app)?;
    Ok(RuntimeLogSettings {
        is_custom: dir != default_dir,
        dir: dir.to_string_lossy().to_string(),
        default_dir: default_dir.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn get_runtime_log_settings(app: AppHandle) -> Result<RuntimeLogSettings, String> {
    settings_snapshot(&app)
}

/// `dir` 为空时恢复默认目录；切换后新日志写入新目录，已有日志留在原位置。
#[tauri::command]
pub fn set_runtime_log_dir(
    app: AppHandle,
    dir: Option<String>,
) -> Result<RuntimeLogSettings, String> {
    let next = logging::set_dir(&app, dir)?;
    log::info!(
        target: "app",
        "运行日志目录已切换到 {}",
        next.to_string_lossy()
    );
    settings_snapshot(&app)
}

#[tauri::command]
pub async fn open_runtime_log_dir(app: AppHandle) -> Result<(), String> {
    let dir = logging::log_dir_or_app(&app)?;
    let path = dir.to_string_lossy().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        // 日志目录名以 .app 结尾，直接交给 open 会被当成应用包并报损坏；改为在 Finder 中定位日志文件。
        #[cfg(target_os = "macos")]
        let status = {
            let target = [LOG_FILE, BACKUP_FILE]
                .iter()
                .map(|name| std::path::Path::new(&path).join(name))
                .find(|candidate| candidate.is_file())
                .unwrap_or_else(|| std::path::PathBuf::from(&path));
            std::process::Command::new("open")
                .arg("-R")
                .arg(&target)
                .status()
        };
        #[cfg(target_os = "windows")]
        let status = std::process::Command::new("explorer").arg(&path).status();
        #[cfg(all(unix, not(target_os = "macos")))]
        let status = std::process::Command::new("xdg-open").arg(&path).status();
        status
            .map_err(|error| format!("打开日志目录失败：{error}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "打开日志目录失败，请手动打开该目录".to_string())
    })
    .await
    .map_err(|error| format!("打开日志目录失败：{error}"))?
}

/// 前端把操作过程写入同一份运行日志，便于把界面现象和后台步骤对应起来。
#[tauri::command]
pub fn log_client_event(level: String, target: Option<String>, message: String) -> Result<(), String> {
    let target = target.unwrap_or_else(|| "ui".to_string());
    let target = target
        .chars()
        .filter(|ch| !ch.is_control() && *ch != ' ')
        .take(80)
        .collect::<String>();
    match level.trim().to_ascii_lowercase().as_str() {
        "warn" | "warning" => log::warn!(target: "ui", "{} - {}", target, message),
        "info" => log::info!(target: "ui", "{} - {}", target, message),
        "debug" => log::debug!(target: "ui", "{} - {}", target, message),
        _ => log::error!(target: "ui", "{} - {}", target, message),
    }
    Ok(())
}
