//! 运行日志：统一收集应用自身的诊断信息。
//!
//! - 只写入应用日志目录，不上传。
//! - 当前文件超过 5 MB 时轮转为 `runtime.1.log`，两个文件合计不超过 10 MB。
//! - 分为正常（INFO）、警告（WARN）、错误（ERROR）三级，供界面筛选。
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender, SyncSender};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use tauri::{AppHandle, Manager};

pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 10 * 1024 * 1024;
pub const LOG_FILE: &str = "runtime.log";
pub const BACKUP_FILE: &str = "runtime.1.log";
/// 自定义日志目录记录在应用配置目录，便于界面与 MCP 读取到同一位置。
pub const SETTINGS_FILE: &str = "log-settings.json";
const MAX_MESSAGE_BYTES: usize = 8 * 1024;
const CHANNEL_CAPACITY: usize = 4096;

enum LogCommand {
    Line(String),
    Clear(Sender<()>),
    Relocate(PathBuf, Sender<Result<(), String>>),
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogSettings {
    #[serde(default)]
    dir: Option<String>,
}

static SENDER: OnceLock<SyncSender<LogCommand>> = OnceLock::new();
static LOG_DIR: OnceLock<RwLock<PathBuf>> = OnceLock::new();

struct RuntimeLogger;

impl log::Log for RuntimeLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(record.level(), record.target(), &record.args().to_string());
        send(LogCommand::Line(line));
    }

    fn flush(&self) {}
}

fn send(command: LogCommand) {
    if let Some(sender) = SENDER.get() {
        // 写入线程异常或队列已满时丢弃日志，绝不阻塞界面操作。
        let _ = sender.try_send(command);
    }
}

fn format_line(level: log::Level, target: &str, message: &str) -> String {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let level = match level {
        log::Level::Error => "ERROR",
        log::Level::Warn => "WARN",
        log::Level::Info => "INFO",
        log::Level::Debug => "DEBUG",
        log::Level::Trace => "TRACE",
    };
    let message = sanitize_message(message);
    format!("{timestamp} {level} {target} - {message}\n")
}

/// 单条日志保持一行，避免错误信息里的换行破坏日志文件结构。
fn sanitize_message(message: &str) -> String {
    let mut clean = message.replace(['\r', '\n'], " ").trim().to_string();
    clean = redact_urls(&clean);
    if clean.len() > MAX_MESSAGE_BYTES {
        let mut end = MAX_MESSAGE_BYTES;
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        clean.truncate(end);
        clean.push_str("…（已截断）");
    }
    clean
}

/// 日志里不保留下载地址的查询参数（可能包含临时签名）。
fn redact_urls(message: &str) -> String {
    message
        .split_whitespace()
        .map(|word| {
            if let Some(position) = word.find('?') {
                let prefix = &word[..position];
                if prefix.starts_with("http://") || prefix.starts_with("https://") {
                    return format!("{prefix}?…");
                }
            }
            word.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 日志中使用的安全地址：只保留协议、主机和路径。
pub fn safe_url(url: &str) -> String {
    match reqwest::Url::parse(url.trim()) {
        Ok(mut parsed) => {
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => "无效地址".to_string(),
    }
}

pub fn init(app: &AppHandle) -> Result<(), String> {
    // 自定义目录不可用时回落到默认目录，避免因为目录问题导致应用无法记录诊断信息。
    let dir = match read_settings(app) {
        Some(custom)
            if fs::create_dir_all(&custom).is_ok() && ensure_writable(&custom).is_ok() =>
        {
            custom
        }
        _ => default_dir(app)?,
    };
    fs::create_dir_all(&dir).map_err(|error| format!("创建日志目录失败：{error}"))?;
    let _ = LOG_DIR.set(RwLock::new(dir.clone()));

    let (sender, receiver) = std::sync::mpsc::sync_channel(CHANNEL_CAPACITY);
    std::thread::Builder::new()
        .name("runtime-log".to_string())
        .spawn(move || writer_loop(dir, receiver))
        .map_err(|error| format!("启动日志写入线程失败：{error}"))?;
    let _ = SENDER.set(sender);

    log::set_boxed_logger(Box::new(RuntimeLogger))
        .map_err(|error| format!("初始化日志失败：{error}"))?;
    let level = parse_level(
        &std::env::var("DEPLOY_DESKTOP_LOG_LEVEL").unwrap_or_default(),
    )?
    .unwrap_or(log::LevelFilter::Info);
    log::set_max_level(level);
    Ok(())
}

fn parse_level(level: &str) -> Result<Option<log::LevelFilter>, String> {
    let filter = match level.trim().to_ascii_lowercase().as_str() {
        "" => return Ok(None),
        "debug" => log::LevelFilter::Debug,
        "info" => log::LevelFilter::Info,
        "warn" | "warning" => log::LevelFilter::Warn,
        "error" => log::LevelFilter::Error,
        other => return Err(format!("不支持的日志级别：{other}")),
    };
    Ok(Some(filter))
}

/// 当前生效的日志级别，供界面展示与切换。
pub fn current_level() -> String {
    match log::max_level() {
        log::LevelFilter::Off => "off",
        log::LevelFilter::Error => "error",
        log::LevelFilter::Warn => "warn",
        log::LevelFilter::Info => "info",
        _ => "debug",
    }
    .to_string()
}

/// 运行期切换日志级别，打开详细调试后可以看到每一次操作的完整过程。
pub fn set_level(level: &str) -> Result<String, String> {
    let filter = parse_level(level)?.ok_or_else(|| "日志级别不能为空".to_string())?;
    log::set_max_level(filter);
    Ok(current_level())
}

pub fn log_dir() -> Option<PathBuf> {
    LOG_DIR
        .get()
        .map(|lock| lock.read().unwrap_or_else(|error| error.into_inner()).clone())
}

/// 应用默认日志目录，用于界面展示与恢复默认。
pub fn default_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_log_dir()
        .map_err(|error| format!("读取日志目录失败：{error}"))
}

pub fn log_dir_or_app(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(dir) = log_dir() {
        return Ok(dir);
    }
    let dir = default_dir(app)?;
    fs::create_dir_all(&dir).map_err(|error| format!("创建日志目录失败：{error}"))?;
    Ok(dir)
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(crate::core::config::app_config_dir(app)?.join(SETTINGS_FILE))
}

fn read_settings(app: &AppHandle) -> Option<PathBuf> {
    read_settings_file(&settings_path(app).ok()?)
}

/// 读取设置文件；内容损坏、目录为空或不是绝对路径时一律视为未设置。
fn read_settings_file(path: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(path).ok()?;
    let settings: LogSettings = serde_json::from_str(&text).ok()?;
    let dir = settings.dir?;
    let dir = dir.trim();
    if dir.is_empty() {
        return None;
    }
    let path = PathBuf::from(dir);
    path.is_absolute().then_some(path)
}

fn persist_settings(app: &AppHandle, dir: Option<&Path>) -> Result<(), String> {
    persist_settings_file(&settings_path(app)?, dir)
}

/// 原子替换设置文件，避免中途失败留下半个 JSON 让下次启动读不到目录。
fn persist_settings_file(path: &Path, dir: Option<&Path>) -> Result<(), String> {
    let text = match dir {
        Some(dir) => serde_json::to_string_pretty(&LogSettings {
            dir: Some(dir.to_string_lossy().to_string()),
        })
        .map_err(|error| format!("保存日志设置失败：{error}"))?,
        None => "{}".to_string(),
    };
    crate::core::atomic_file::write_text_atomic(
        path,
        &text,
        crate::core::atomic_file::FilePermission::Default,
    )
    .map_err(|error| format!("保存日志设置失败：{error}"))
}

fn ensure_writable(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|error| format!("无法创建日志目录：{error}"))?;
    let probe = dir.join(".deploy-desktop-write-test");
    fs::write(&probe, b"ok").map_err(|error| format!("日志目录不可写入：{error}"))?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// 切换日志目录；`None` 表示恢复应用默认目录。切换后新日志写入新目录，旧日志保持原位。
pub fn set_dir(app: &AppHandle, dir: Option<String>) -> Result<PathBuf, String> {
    let is_custom = dir.is_some();
    let target = match dir {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err("日志目录不能为空".to_string());
            }
            let path = PathBuf::from(trimmed);
            if !path.is_absolute() {
                return Err("日志目录必须是绝对路径".to_string());
            }
            path
        }
        None => default_dir(app)?,
    };
    ensure_writable(&target)?;
    if let Some(sender) = SENDER.get() {
        let (ack, wait) = channel();
        sender
            .try_send(LogCommand::Relocate(target.clone(), ack))
            .map_err(|_| "日志写入繁忙，请稍后重试".to_string())?;
        wait.recv_timeout(Duration::from_secs(3))
            .map_err(|_| "切换日志目录超时，请稍后重试".to_string())??;
    }
    if let Some(lock) = LOG_DIR.get() {
        *lock.write().unwrap_or_else(|error| error.into_inner()) = target.clone();
    }
    persist_settings(app, if is_custom { Some(target.as_path()) } else { None })?;
    Ok(target)
}

/// 清空日志并等待写入线程处理完成，避免后续写入落到已截断的旧偏移。
pub fn clear() -> Result<(), String> {
    let Some(sender) = SENDER.get() else {
        return Ok(());
    };
    let (ack, wait) = channel();
    sender
        .try_send(LogCommand::Clear(ack))
        .map_err(|_| "日志写入繁忙，请稍后重试".to_string())?;
    wait.recv_timeout(Duration::from_secs(3))
        .map_err(|_| "清空日志超时，请稍后重试".to_string())
}

pub fn total_bytes(dir: &Path) -> u64 {
    [LOG_FILE, BACKUP_FILE]
        .iter()
        .filter_map(|name| fs::metadata(dir.join(name)).ok())
        .map(|meta| meta.len())
        .sum()
}

fn writer_loop(dir: PathBuf, receiver: Receiver<LogCommand>) {
    let mut sink = match Sink::open(&dir) {
        Ok(sink) => sink,
        Err(_) => return,
    };
    while let Ok(command) = receiver.recv() {
        match command {
            LogCommand::Line(line) => {
                let _ = sink.write(&line);
            }
            LogCommand::Clear(ack) => {
                let _ = sink.clear();
                let _ = ack.send(());
            }
            LogCommand::Relocate(dir, ack) => {
                let result = Sink::open(&dir).map(|next| sink = next).map_err(|error| {
                    format!("无法写入日志目录：{error}")
                });
                let _ = ack.send(result);
            }
        }
    }
}

struct Sink {
    dir: PathBuf,
    file: File,
    size: u64,
    max_file_bytes: u64,
}

impl Sink {
    fn open(dir: &Path) -> io::Result<Self> {
        Self::open_with_limit(dir, MAX_FILE_BYTES)
    }

    fn open_with_limit(dir: &Path, max_file_bytes: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(LOG_FILE);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut size = file.metadata()?.len();
        let mut sink = Self {
            dir: dir.to_path_buf(),
            file,
            size,
            max_file_bytes,
        };
        if size >= max_file_bytes {
            sink.rotate()?;
            size = 0;
            sink.size = size;
        }
        Ok(sink)
    }

    fn write(&mut self, line: &str) -> io::Result<()> {
        let bytes = line.as_bytes();
        if self.size > 0 && self.size + bytes.len() as u64 > self.max_file_bytes {
            self.rotate()?;
        }
        self.file.write_all(bytes)?;
        // 逐条刷新，应用异常退出时也能保留最近的诊断信息。
        self.file.flush()?;
        self.size += bytes.len() as u64;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        let current = self.dir.join(LOG_FILE);
        let backup = self.dir.join(BACKUP_FILE);
        self.file.flush()?;
        let _ = fs::remove_file(&backup);
        fs::rename(&current, &backup)?;
        self.file = OpenOptions::new().create(true).append(true).open(&current)?;
        self.size = 0;
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.file.set_len(0)?;
        self.size = 0;
        let _ = fs::remove_file(self.dir.join(BACKUP_FILE));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("deploy-log-{tag}-{}-{stamp}", std::process::id()))
    }

    #[test]
    fn rotation_keeps_total_size_within_two_files() {
        let dir = temp_dir("rotate");
        let mut sink = Sink::open_with_limit(&dir, 1024).unwrap();
        for _ in 0..40 {
            sink.write(&format!("{}\n", "x".repeat(100))).unwrap();
        }
        let current = fs::metadata(dir.join(LOG_FILE)).unwrap().len();
        let backup = fs::metadata(dir.join(BACKUP_FILE)).unwrap().len();
        assert!(current <= 1024);
        assert!(backup <= 1024);
        assert!(current + backup <= 2 * 1024);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_resets_current_file_and_removes_backup() {
        let dir = temp_dir("clear");
        let mut sink = Sink::open_with_limit(&dir, 256).unwrap();
        for _ in 0..10 {
            sink.write(&format!("{}\n", "y".repeat(100))).unwrap();
        }
        assert!(dir.join(BACKUP_FILE).exists());
        sink.clear().unwrap();
        assert_eq!(fs::metadata(dir.join(LOG_FILE)).unwrap().len(), 0);
        assert!(!dir.join(BACKUP_FILE).exists());
        sink.write("after clear\n").unwrap();
        let content = fs::read_to_string(dir.join(LOG_FILE)).unwrap();
        assert_eq!(content, "after clear\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn settings_round_trip_keeps_absolute_directory() {
        let dir = temp_dir("settings-rt");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(SETTINGS_FILE);
        persist_settings_file(&file, Some(&dir)).unwrap();
        assert_eq!(read_settings_file(&file), Some(dir.clone()));

        persist_settings_file(&file, None).unwrap();
        assert_eq!(read_settings_file(&file), None);
        let raw = fs::read_to_string(&file).unwrap();
        assert_eq!(raw.trim(), "{}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn settings_ignore_relative_or_broken_directories() {
        let dir = temp_dir("settings-bad");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(SETTINGS_FILE);

        fs::write(&file, r#"{"dir":"./logs"}"#).unwrap();
        assert_eq!(read_settings_file(&file), None);

        fs::write(&file, r#"{"dir":"   "}"#).unwrap();
        assert_eq!(read_settings_file(&file), None);

        fs::write(&file, "not json").unwrap();
        assert_eq!(read_settings_file(&file), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn message_is_single_line_and_drops_url_query() {
        let line = format_line(
            log::Level::Error,
            "artifact",
            "下载失败\nhttps://example.com/a.tgz?token=secret&sig=1",
        );
        assert!(!line[..line.len() - 1].contains('\n'));
        assert!(line.contains("https://example.com/a.tgz?…"));
        assert!(!line.contains("secret"));
    }
}
