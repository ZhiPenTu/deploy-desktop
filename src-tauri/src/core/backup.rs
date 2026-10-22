//! 本机备份与「先暂存、再替换」的共用实现。
//!
//! 更新部署目录和安装交付包都要用同一套动作：把新内容放在同级的临时位置，再把
//! 旧内容改名让位，最后把临时内容改名到目标。中途失败必须换回旧内容，否则目录会
//! 停在既没有旧版本也没有新版本的状态。这里只负责这段文件系统动作，备份目录命名
//! 与面向用户的提示仍由命令层决定。

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 进程号、时间与自增序号，保证同一次运行内生成的临时名互不冲突。
pub fn unique_suffix() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// 删除文件或目录；目标已不存在时同样视为成功。
pub fn remove_path(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(directory_error) if directory_error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(directory_error) => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(directory_error),
        },
    }
}

/// 替换失败发生的阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapStage {
    /// 把已有内容改名为上一版失败，目标保持原样。
    MoveAside,
    /// 把暂存内容落位失败。
    Install,
}

/// 替换失败的细节。命令层据此决定提示文案。
#[derive(Debug)]
pub struct SwapFailure {
    pub stage: SwapStage,
    pub error: io::Error,
    /// 落位失败时旧内容的回滚结果；没有旧内容时为 `Ok(())`。
    pub rollback: io::Result<()>,
}

/// 用 `staged` 替换 `target`，旧内容先改名为 `previous`。
///
/// 返回此前是否存在旧内容。落位失败时已尝试把旧内容改回 `target`，调用方必须检查
/// [`SwapFailure::rollback`]：回滚也失败说明目标目录需要人工处理。
pub fn swap_staged(staged: &Path, target: &Path, previous: &Path) -> Result<bool, SwapFailure> {
    let had_previous = target.exists();
    if had_previous {
        if let Err(error) = fs::rename(target, previous) {
            return Err(SwapFailure {
                stage: SwapStage::MoveAside,
                error,
                rollback: Ok(()),
            });
        }
    }
    if let Err(error) = fs::rename(staged, target) {
        let rollback = if had_previous {
            fs::rename(previous, target)
        } else {
            Ok(())
        };
        return Err(SwapFailure {
            stage: SwapStage::Install,
            error,
            rollback,
        });
    }
    Ok(had_previous)
}

/// 替换成功后清理旧内容。失败只留下临时目录，不影响已完成的替换。
pub fn discard_previous(previous: &Path, had_previous: bool) -> io::Result<()> {
    if had_previous {
        remove_path(previous)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deploy-desktop-core-backup-{label}-{}",
            unique_suffix()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn suffix_is_unique_within_one_process() {
        let first = unique_suffix();
        let second = unique_suffix();
        assert_ne!(first, second);
        assert!(first.starts_with(&std::process::id().to_string()));
    }

    #[test]
    fn swap_replaces_target_and_reports_previous() {
        let dir = temp_dir("swap");
        let staged = dir.join("staged");
        let target = dir.join("target");
        let previous = dir.join("previous");
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("version"), "new").unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("version"), "old").unwrap();

        assert!(swap_staged(&staged, &target, &previous).unwrap());
        assert_eq!(fs::read_to_string(target.join("version")).unwrap(), "new");
        assert!(previous.join("version").is_file());
        assert!(!staged.exists());

        discard_previous(&previous, true).unwrap();
        assert!(!previous.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn swap_without_previous_leaves_nothing_to_restore() {
        let dir = temp_dir("fresh");
        let staged = dir.join("staged");
        let target = dir.join("target");
        let previous = dir.join("previous");
        fs::write(&staged, "new").unwrap();

        assert!(!swap_staged(&staged, &target, &previous).unwrap());
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert!(!previous.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_staged_content_restores_previous_target() {
        let dir = temp_dir("rollback");
        let staged = dir.join("absent-staged");
        let target = dir.join("target");
        let previous = dir.join("previous");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("version"), "old").unwrap();

        let failure = swap_staged(&staged, &target, &previous).unwrap_err();
        assert_eq!(failure.stage, SwapStage::Install);
        assert!(failure.rollback.is_ok());
        assert_eq!(fs::read_to_string(target.join("version")).unwrap(), "old");
        assert!(!previous.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_path_handles_files_directories_and_absent_targets() {
        let dir = temp_dir("remove");
        let file = dir.join("file");
        let nested = dir.join("nested/deeper");
        fs::write(&file, "x").unwrap();
        fs::create_dir_all(&nested).unwrap();

        remove_path(&file).unwrap();
        remove_path(&dir.join("nested")).unwrap();
        remove_path(&dir.join("never-existed")).unwrap();

        assert!(!file.exists());
        assert!(!dir.join("nested").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
