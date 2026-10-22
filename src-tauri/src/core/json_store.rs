//! JSON 配置文件读写。
//!
//! 读取时区分「文件不存在」与「内容损坏」：前者交给调用方决定默认值，后者先把原文件
//! 备份再返回错误，避免后续保存把用户仅存的现场覆盖掉。

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::atomic_file::{self, FilePermission};
use super::file_lock;

/// 读取 JSON 配置；文件不存在返回 `Ok(None)`。
pub fn load<T: DeserializeOwned>(path: &Path, label: &str) -> Result<Option<T>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取{label}失败：{error}")),
    };
    if content.trim().is_empty() {
        let backup = backup_corrupt(path);
        return Err(corrupt_error(label, backup.as_deref()));
    }
    match serde_json::from_str::<T>(&content) {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            log::warn!("{label}解析失败：{error}");
            let backup = backup_corrupt(path);
            Err(corrupt_error(label, backup.as_deref()))
        }
    }
}

/// 整体覆盖写入，持锁期间其他进程不会读到半成品，也不会与本进程交错写入。
pub fn save<T: Serialize>(
    path: &Path,
    value: &T,
    label: &str,
    permission: FilePermission,
) -> Result<(), String> {
    let _guard = file_lock::acquire(path)?;
    write_unlocked(path, value, label, permission)
}

/// 在锁内完成「读取 → 修改 → 写入」。
///
/// 桌面端与 MCP 服务会改写同一份配置，先按最新磁盘内容修改再写回，
/// 避免用内存里的旧快照覆盖对方刚写入的变更。文件不存在时从 `initial` 开始。
pub fn update<T, F>(
    path: &Path,
    initial: T,
    label: &str,
    permission: FilePermission,
    mutate: F,
) -> Result<T, String>
where
    T: DeserializeOwned + Serialize,
    F: FnOnce(&mut T) -> Result<(), String>,
{
    let _guard = file_lock::acquire(path)?;
    let mut current = match load(path, label)? {
        Some(value) => value,
        None => initial,
    };
    mutate(&mut current)?;
    write_unlocked(path, &current, label, permission)?;
    Ok(current)
}

fn write_unlocked<T: Serialize>(
    path: &Path,
    value: &T,
    label: &str,
    permission: FilePermission,
) -> Result<(), String> {
    let content = serde_json::to_string_pretty(value)
        .map_err(|error| format!("生成{label}内容失败：{error}"))?;
    atomic_file::write_text_atomic(path, &content, permission)
        .map_err(|error| format!("保存{label}失败：{error}"))
}

fn corrupt_error(label: &str, backup: Option<&Path>) -> String {
    match backup {
        Some(path) => format!(
            "{label}已损坏，无法读取。原文件已备份为 {}，请修复或恢复该文件后重试。",
            path.display()
        ),
        None => format!("{label}已损坏，无法读取。请检查该文件或恢复本机配置备份后重试。"),
    }
}

/// 把损坏的原文件改名备份；改名失败时退化为复制。无法备份时返回 `None`。
fn backup_corrupt(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    for attempt in 0..16 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let candidate = parent.join(format!("{name}.corrupt-{stamp}{suffix}"));
        if candidate.exists() {
            continue;
        }
        if std::fs::rename(path, &candidate).is_ok() {
            return Some(candidate);
        }
        if std::fs::copy(path, &candidate).is_ok() {
            return Some(candidate);
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    struct Sample {
        name: String,
        count: u32,
    }

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "deploy-desktop-core-json-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn backups(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().contains(".corrupt-"))
            .collect()
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = temp_dir("missing");
        let loaded = load::<Sample>(&dir.join("sample.json"), "测试配置").unwrap();
        assert!(loaded.is_none());
        assert!(backups(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trips_values() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("sample.json");
        let value = Sample {
            name: "部署舱".to_string(),
            count: 3,
        };

        save(&path, &value, "测试配置", FilePermission::Default).unwrap();
        let loaded = load::<Sample>(&path, "测试配置").unwrap();
        assert_eq!(loaded, Some(value));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_backed_up_before_reporting() {
        let dir = temp_dir("corrupt");
        let path = dir.join("sample.json");
        fs::write(&path, "{\"name\": ").unwrap();

        let error = load::<Sample>(&path, "测试配置").unwrap_err();
        assert!(error.contains("已损坏"), "unexpected message: {error}");
        assert!(!path.exists(), "损坏文件应被移出原位置");

        let backups = backups(&dir);
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "{\"name\": ");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_reads_latest_disk_state_under_lock() {
        let dir = temp_dir("update");
        let path = dir.join("sample.json");

        let created = update(
            &path,
            Sample::default(),
            "测试配置",
            FilePermission::Default,
            |sample| {
                sample.name = "第一次".to_string();
                sample.count = 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(created.count, 1);

        // 另一进程（此处直接写盘）追加变更，update 应基于磁盘最新内容累加。
        fs::write(&path, r#"{"name":"外部","count":10}"#).unwrap();
        let merged = update(
            &path,
            Sample::default(),
            "测试配置",
            FilePermission::Default,
            |sample| {
                sample.count += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(merged.name, "外部");
        assert_eq!(merged.count, 11);

        // 写入完成后锁文件应被释放。
        assert!(!file_lock::lock_path(&path).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_keeps_file_intact_when_mutation_fails() {
        let dir = temp_dir("update-fail");
        let path = dir.join("sample.json");
        let original = Sample {
            name: "原始".to_string(),
            count: 7,
        };
        save(&path, &original, "测试配置", FilePermission::Default).unwrap();

        let error = update(
            &path,
            Sample::default(),
            "测试配置",
            FilePermission::Default,
            |sample| {
                sample.count = 99;
                Err("校验失败".to_string())
            },
        )
        .unwrap_err();
        assert_eq!(error, "校验失败");
        let reloaded = load::<Sample>(&path, "测试配置").unwrap().unwrap();
        assert_eq!(reloaded, original);
        assert!(!file_lock::lock_path(&path).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_file_is_treated_as_corrupt() {
        let dir = temp_dir("empty");
        let path = dir.join("sample.json");
        fs::write(&path, "   ").unwrap();

        assert!(load::<Sample>(&path, "测试配置").is_err());
        assert_eq!(backups(&dir).len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }
}
