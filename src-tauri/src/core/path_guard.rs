//! 路径安全性判断。
//!
//! 交付包描述符、用户填写的目录和远程返回的清单都会带来路径。拼接之前必须先拒绝
//! 绝对路径、`..` 跳转与分隔符混用。这里只放与业务无关的纯判断：不访问磁盘、
//! 不解析符号链接，也不生成面向用户的文案，由命令层决定提示内容。

use std::path::{Component, Path, PathBuf};

/// 把 Windows 扩展长度路径转换成 Docker CLI 接受的普通路径。
///
/// `fs::canonicalize` 在 Windows 上返回 `\\?\D:\...` 或 `\\?\UNC\...`。
/// Docker 的 bind mount 不接受这种前缀，相对挂载会因此被解析成无法挂载的地址。
/// 只还原盘符路径与 UNC 路径：`\\?\Volume{...}` 和 `\\?\GLOBALROOT\...` 没有等价的
/// 普通路径，保留原样交给上层判断，避免变成无效的相对路径。非 Windows 或普通路径原样返回。
pub fn docker_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let Some(rest) = text.strip_prefix(r"\\?\") else {
        return path.to_path_buf();
    };
    if rest
        .get(..4)
        .map(|prefix| prefix.eq_ignore_ascii_case("UNC\\"))
        .unwrap_or(false)
    {
        return PathBuf::from(format!(r"\\{}", &rest[4..]));
    }
    let bytes = rest.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

/// 按路径组件拆分路径，便于逐段比较。
///
/// 组件拆分由平台决定分隔符，Unix 上反斜杠是普通文件名字符，不会被当成层级。
fn path_segments(path: &Path) -> Vec<String> {
    docker_path(path)
        .components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => {
                Some(prefix.as_os_str().to_string_lossy().to_uppercase())
            }
            Component::RootDir => Some(String::from("/")),
            Component::CurDir => None,
            Component::ParentDir => Some(String::from("..")),
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
        })
        .collect()
}

/// 按路径段判断 `target` 是否位于 `base` 内；Windows 下忽略大小写。
pub fn path_is_within(base: &Path, target: &Path) -> bool {
    let base = path_segments(base);
    let target = path_segments(target);
    if base.is_empty() || target.len() < base.len() {
        return false;
    }
    // 含 `..` 的路径无法在不访问磁盘的情况下判定归属，直接视为不包含。
    if base.iter().any(|part| part == "..") || target.iter().any(|part| part == "..") {
        return false;
    }
    base.iter().zip(target.iter()).all(|(left, right)| {
        if cfg!(windows) {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    })
}

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

    #[test]
    fn docker_path_removes_windows_verbatim_prefix() {
        assert_eq!(
            docker_path(Path::new(r"\\?\D:\400mbushu\deploy")),
            PathBuf::from(r"D:\400mbushu\deploy")
        );
        assert_eq!(
            docker_path(Path::new(r"\\?\UNC\server\share\app")),
            PathBuf::from(r"\\server\share\app")
        );
        assert_eq!(
            docker_path(Path::new("/opt/demo")),
            PathBuf::from("/opt/demo")
        );
        assert_eq!(
            docker_path(Path::new(r"\\?\unc\server\share\app")),
            PathBuf::from(r"\\server\share\app")
        );
        assert_eq!(
            docker_path(Path::new(r"\\?\Volume{1234}\app")),
            PathBuf::from(r"\\?\Volume{1234}\app")
        );
    }

    #[test]
    fn docker_path_keeps_namespaces_without_plain_equivalent() {
        assert_eq!(
            docker_path(Path::new(r"\\?\GLOBALROOT\Device\HarddiskVolume1\app")),
            PathBuf::from(r"\\?\GLOBALROOT\Device\HarddiskVolume1\app")
        );
        assert_eq!(
            docker_path(Path::new(r"\\?\Volume{1234}\app")),
            PathBuf::from(r"\\?\Volume{1234}\app")
        );
    }

    #[test]
    fn path_containment_does_not_match_sibling_prefixes() {
        assert!(path_is_within(
            Path::new("/opt/demo"),
            Path::new("/opt/demo/app")
        ));
        assert!(!path_is_within(
            Path::new("/opt/demo"),
            Path::new("/opt/demo2")
        ));
        assert!(path_is_within(Path::new("/"), Path::new("/opt/demo")));
    }

    #[test]
    fn path_containment_normalizes_repeated_separators() {
        assert!(path_is_within(
            Path::new("/opt/demo"),
            Path::new("/opt//demo/app")
        ));
        assert!(path_is_within(
            Path::new("/opt/demo/"),
            Path::new("/opt/demo/app")
        ));
    }

    #[test]
    fn path_containment_rejects_parent_segments() {
        assert!(!path_is_within(
            Path::new("/opt/demo"),
            Path::new("/opt/demo/../demo2")
        ));
        assert!(!path_is_within(
            Path::new("/opt/demo/.."),
            Path::new("/opt/demo2")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn unix_path_containment_keeps_backslash_inside_names() {
        assert!(!path_is_within(
            Path::new("/opt/demo"),
            Path::new("/opt/demo\\child")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_containment_ignores_case() {
        assert!(path_is_within(
            Path::new(r"D:\400mbushu\deploy"),
            Path::new(r"d:\400MBUSHU\deploy\site")
        ));
        assert!(!path_is_within(
            Path::new(r"D:\400mbushu\deploy"),
            Path::new(r"D:\400mbushu\deploy2")
        ));
    }
}
