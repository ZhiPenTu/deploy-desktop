//! 配置文件跨进程互斥。
//!
//! 同一份配置文件可能同时被桌面端与 MCP 服务写入（项目、制品源、令牌与版本缓存）。
//! 两个进程各自读改写时会互相覆盖，这里用「相邻锁文件 + 独占创建」把读改写变成串行：
//! 只有抢到 `<文件>.lock` 的一方能读、改、写，另一方等待或得到明确的重试提示。
//!
//! 选择锁文件而不是系统文件锁，是因为锁文件在两端都用标准库就能实现，MCP 服务
//! 与桌面端遵守同一套约定即可。进程异常退出会留下锁文件，因此超过 [`STALE_AFTER`]
//! 未被复用的锁会被视为残留并清理。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

const LOCK_SUFFIX: &str = ".lock";
const RETRY_INTERVAL: Duration = Duration::from_millis(15);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
const STALE_AFTER: Duration = Duration::from_secs(30);
/// Windows 上删除文件的瞬间新建同名文件、或杀毒软件扫描新建文件，都会返回拒绝访问。
/// 这类情况短暂重试即可恢复，不应直接让用户失败。
const DENIED_RETRY_WINDOW: Duration = Duration::from_millis(500);

/// 持有期间保证同一份配置文件只被当前进程改写；离开作用域即释放。
#[derive(Debug)]
pub struct FileLock {
    path: PathBuf,
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// 抢占配置锁；超时后返回可展示给用户的重试提示。
pub fn acquire(target: &Path) -> Result<FileLock, String> {
    acquire_with_timeout(target, DEFAULT_TIMEOUT)
}

/// 锁文件路径与目标文件相邻，保证同一目录内的写入方看到同一把锁。
pub fn lock_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(LOCK_SUFFIX);
    PathBuf::from(name)
}

fn acquire_with_timeout(target: &Path, timeout: Duration) -> Result<FileLock, String> {
    let path = lock_path(target);
    let deadline = Instant::now() + timeout;
    let mut denied_since: Option<Instant> = None;
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                return Ok(FileLock { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                denied_since = None;
                if is_stale(&path) {
                    let _ = fs::remove_file(&path);
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err("配置正在被另一个程序写入，请稍后重试".to_string());
                }
                sleep(RETRY_INTERVAL);
            }
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                let started = *denied_since.get_or_insert_with(Instant::now);
                if started.elapsed() >= DENIED_RETRY_WINDOW || Instant::now() >= deadline {
                    return Err(format!("无法创建配置锁：{error}"));
                }
                sleep(RETRY_INTERVAL);
            }
            Err(error) => return Err(format!("无法创建配置锁：{error}")),
        }
    }
}

fn is_stale(path: &Path) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age > STALE_AFTER)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "deploy-desktop-file-lock-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lock_is_released_on_drop() {
        let dir = temp_dir("release");
        let target = dir.join("sample.json");
        let lock = acquire(&target).unwrap();
        assert!(lock_path(&target).exists());
        drop(lock);
        assert!(!lock_path(&target).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_writers_are_serialized() {
        let dir = temp_dir("serial");
        let target = dir.join("sample.json");
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let target = target.clone();
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            handles.push(std::thread::spawn(move || {
                let _lock = acquire(&target).unwrap();
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                sleep(Duration::from_millis(20));
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1, "同一时刻只应有一个写入方");
        assert!(!lock_path(&target).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn abandoned_lock_is_reclaimed() {
        let dir = temp_dir("stale");
        let target = dir.join("sample.json");
        let lock = lock_path(&target);
        fs::write(&lock, "1\n").unwrap();
        let old = SystemTime::now() - Duration::from_secs(120);
        set_modified(&lock, old);

        let guard = acquire_with_timeout(&target, Duration::from_millis(200)).unwrap();
        assert!(lock.exists());
        drop(guard);
        assert!(!lock.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn transient_lock_path_obstruction_is_retried() {
        let dir = temp_dir("obstruct");
        let target = dir.join("sample.json");
        let lock = lock_path(&target);
        // 用同名目录占住锁路径：新建会被拒绝或提示已存在，移除后应能拿到锁。
        fs::create_dir_all(&lock).unwrap();
        let blocker = lock.clone();
        let handle = std::thread::spawn(move || {
            sleep(Duration::from_millis(80));
            let _ = fs::remove_dir_all(&blocker);
        });
        let acquired = acquire_with_timeout(&target, Duration::from_secs(3));
        handle.join().unwrap();
        let acquired = acquired.expect("锁路径短暂占用后应能重试成功");
        drop(acquired);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn timeout_reports_retryable_error() {
        let dir = temp_dir("timeout");
        let target = dir.join("sample.json");
        fs::write(lock_path(&target), "1\n").unwrap();

        let error = acquire_with_timeout(&target, Duration::from_millis(60)).unwrap_err();
        assert!(error.contains("稍后重试"), "unexpected message: {error}");
        let _ = fs::remove_dir_all(&dir);
    }

    fn set_modified(path: &Path, time: SystemTime) {
        let file = OpenOptions::new().write(true).open(path).unwrap();
        let duration = time
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        file.set_modified(time)
            .unwrap_or_else(|error| panic!("无法调整 {path:?} 的时间戳（{duration:?}）：{error}"));
    }
}
