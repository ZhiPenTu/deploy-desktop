//! 路径安全性判断。
//!
//! 交付包描述符、用户填写的目录和远程返回的清单都会带来路径。拼接之前必须先拒绝
//! 绝对路径、`..` 跳转与分隔符混用。这里只放与业务无关的纯判断：不访问磁盘、
//! 不解析符号链接，也不生成面向用户的文案，由命令层决定提示内容。

use std::path::{Component, Path};

/// 相对路径不能直接拼接的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeIssue {
    /// 去掉首尾空白后为空。
    Empty,
    /// 含反斜杠：交付包内统一使用 `/`。
    Backslash,
    /// 当前平台的绝对路径。
    Absolute,
    /// 含 `.`、`..`、根目录或前缀段。
    NonPlain,
}

/// 判断 `value` 能否作为相对路径拼在某个目录之后。
///
/// 只接受由普通段组成的相对路径，`.` 与 `..` 一律拒绝，避免一处写错就写到目录之外。
pub fn check_relative(value: &str) -> Result<(), RelativeIssue> {
    let text = value.trim();
    if text.is_empty() {
        return Err(RelativeIssue::Empty);
    }
    if text.contains('\\') {
        return Err(RelativeIssue::Backslash);
    }
    // 复用与运行平台无关的判断：在 Windows 上 `/x` 也要按绝对路径报错，
    // 而不是被后面的分段检查归为不合法路径。
    if is_absolute(text) {
        return Err(RelativeIssue::Absolute);
    }
    // 与 `Path::components` 保持一致：中间的 `.` 与重复分隔符会被规范化，
    // 因此 `a/./b` 与 `a//b` 仍然可用，开头的 `./` 与任何 `..` 都会被拒绝。
    let mut has_normal = false;
    for part in Path::new(text).components() {
        match part {
            Component::Normal(_) => has_normal = true,
            _ => return Err(RelativeIssue::NonPlain),
        }
    }
    if !has_normal {
        return Err(RelativeIssue::NonPlain);
    }
    Ok(())
}

/// 路径中是否出现 `..` 段。
pub fn has_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|part| matches!(part, Component::ParentDir))
}

/// 是否为当前平台或 Windows 形式的绝对路径。
///
/// 部署目录会被写入主机记录，并可能从另一台机器上读取，因此 Windows 盘符与
/// UNC 形式一律按绝对路径处理，不受运行平台影响。
pub fn is_absolute(value: &str) -> bool {
    if value.starts_with('/') || value.starts_with("\\\\") {
        return true;
    }
    let bytes = value.as_bytes();
    bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// `target` 是否位于 `base` 之内（按路径段比较，不解析符号链接）。
pub fn is_within(base: &Path, target: &Path) -> bool {
    target.starts_with(base)
}

/// `target` 是否位于 `base` 之内且不是 `base` 本身。
pub fn is_strictly_within(base: &Path, target: &Path) -> bool {
    target.starts_with(base) && target != base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_relative_paths_are_accepted() {
        assert_eq!(check_relative("deploy/docker-compose.yml"), Ok(()));
        assert_eq!(check_relative("  app/bin  "), Ok(()));
        assert_eq!(check_relative("site"), Ok(()));
    }

    #[test]
    fn each_unsafe_shape_is_reported() {
        assert_eq!(check_relative("   "), Err(RelativeIssue::Empty));
        assert_eq!(
            check_relative("deploy\\x.yml"),
            Err(RelativeIssue::Backslash)
        );
        assert_eq!(check_relative("/etc/passwd"), Err(RelativeIssue::Absolute));
        assert_eq!(check_relative("../outside"), Err(RelativeIssue::NonPlain));
        assert_eq!(check_relative("a/../../b"), Err(RelativeIssue::NonPlain));
        assert_eq!(check_relative("./a"), Err(RelativeIssue::NonPlain));
        assert_eq!(check_relative("."), Err(RelativeIssue::NonPlain));
        assert_eq!(check_relative("a/./b"), Ok(()));
        assert_eq!(check_relative("a//b"), Ok(()));
    }

    #[test]
    fn parent_dir_detection_ignores_embedded_dots() {
        assert!(has_parent_dir(Path::new("a/../b")));
        assert!(has_parent_dir(Path::new("../b")));
        assert!(!has_parent_dir(Path::new("a/..b/c")));
        assert!(!has_parent_dir(Path::new("a/b")));
    }

    #[test]
    fn absolute_detection_covers_posix_and_windows() {
        assert!(is_absolute("/opt/demo"));
        assert!(is_absolute("C:/demo"));
        assert!(is_absolute("D:\\demo"));
        assert!(is_absolute("\\\\server\\share"));
        assert!(!is_absolute("opt/demo"));
        assert!(!is_absolute("C:demo"));
        assert!(!is_absolute(""));
    }

    #[test]
    fn containment_excludes_base_itself() {
        let base = Path::new("/opt/demo");
        assert!(is_within(base, Path::new("/opt/demo/app")));
        assert!(is_within(base, base));
        assert!(is_strictly_within(base, Path::new("/opt/demo/app")));
        assert!(!is_strictly_within(base, base));
        assert!(!is_within(base, Path::new("/opt/other")));
    }
}
