//! Docker 官方离线安装包目录与下载。
//!
//! 版本列表分别来自 Docker Engine 静态二进制目录和 Docker Desktop 发布说明。
//! 下载地址只按平台、架构、版本与构建号重新拼装，不接受前端传入任意 URL。

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use tauri::ipc::Channel;

const LINUX_STATIC_X86_64: &str = "https://download.docker.com/linux/static/stable/x86_64/";
const LINUX_STATIC_AARCH64: &str = "https://download.docker.com/linux/static/stable/aarch64/";
const DESKTOP_RELEASE_NOTES: &str = "https://docs.docker.com/desktop/release-notes/";
const DESKTOP_VERSION_LIMIT: usize = 40;
const MAX_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DockerOfflinePackage {
    pub platform: String,
    pub arch: String,
    pub version: String,
    pub build: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerOfflineCatalog {
    pub packages: Vec<DockerOfflinePackage>,
    pub default_platform: String,
    pub download_dir: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerDownloadProgress {
    pub stage: String,
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerDownloadResult {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[tauri::command]
pub async fn list_docker_offline_packages() -> Result<DockerOfflineCatalog, String> {
    crate::commands::blocking::run(list_docker_offline_packages_blocking).await
}

#[tauri::command]
pub async fn download_docker_offline_package(
    platform: String,
    arch: String,
    version: String,
    build: String,
    save_path: String,
    on_event: Channel<DockerDownloadProgress>,
) -> Result<DockerDownloadResult, String> {
    crate::commands::blocking::run(move || {
        download_docker_offline_package_blocking(
            platform, arch, version, build, save_path, on_event,
        )
    })
    .await
}

fn list_docker_offline_packages_blocking() -> Result<DockerOfflineCatalog, String> {
    let mut packages = Vec::new();
    let mut warnings = Vec::new();

    for (arch, url) in [
        ("x86_64", LINUX_STATIC_X86_64),
        ("aarch64", LINUX_STATIC_AARCH64),
    ] {
        match fetch_text(url) {
            Ok(html) => packages.extend(parse_linux_index(&html, arch)),
            Err(error) => {
                log::warn!(target: "docker-download", "获取 Linux 静态包版本失败：{arch}：{error}");
                warnings.push(format!("未能获取 Linux（{arch}）版本列表，请稍后重试"));
            }
        }
    }

    match fetch_text(DESKTOP_RELEASE_NOTES) {
        Ok(html) => packages.extend(parse_desktop_release_notes(&html)),
        Err(error) => {
            log::warn!(target: "docker-download", "获取 Docker Desktop 版本失败：{error}");
            warnings.push("未能获取 Windows 与 macOS 版本列表，请稍后重试".to_string());
        }
    }

    normalize_packages(&mut packages);
    if packages.is_empty() {
        return Err("暂时无法获取 Docker 离线包版本列表，请检查网络后重试".to_string());
    }

    let default_platform = default_platform(&packages);
    let download_dir = download_dir();
    Ok(DockerOfflineCatalog {
        packages,
        default_platform,
        download_dir,
        warnings,
    })
}

fn fetch_text(url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("deploy-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| {
            log::error!(target: "docker-download", "创建官方下载客户端失败：{error}");
            "无法连接 Docker 官方下载站，请稍后重试".to_string()
        })?;
    let response = client.get(url).send().map_err(|error| {
        log::warn!(target: "docker-download", "请求官方版本列表失败：{error}");
        "无法连接 Docker 官方下载站，请检查网络后重试".to_string()
    })?;
    if !response.status().is_success() {
        log::warn!(
            target: "docker-download",
            "官方版本列表返回异常状态：{}",
            response.status()
        );
        return Err("Docker 官方下载站暂时不可用，请稍后重试".to_string());
    }
    response.text().map_err(|error| {
        log::warn!(target: "docker-download", "读取官方版本列表失败：{error}");
        "读取 Docker 版本信息失败，请稍后重试".to_string()
    })
}

fn parse_linux_index(html: &str, arch: &str) -> Vec<DockerOfflinePackage> {
    let mut seen = HashSet::new();
    let mut packages = Vec::new();
    let mut rest = html;
    while let Some(marker) = rest.find("href=\"") {
        let start = marker + "href=\"".len();
        let Some(end) = rest[start..].find('"') else {
            break;
        };
        let href = rest[start..start + end].trim();
        if let Some(version) = linux_version_from_filename(href) {
            if seen.insert(version.clone()) {
                packages.push(DockerOfflinePackage {
                    platform: "linux".to_string(),
                    arch: arch.to_string(),
                    build: String::new(),
                    file_name: format!("docker-{version}.tgz"),
                    version,
                });
            }
        }
        rest = &rest[start + end + 1..];
    }
    packages
}

fn linux_version_from_filename(name: &str) -> Option<String> {
    let name = name.rsplit('/').next()?;
    let version = name.strip_prefix("docker-")?.strip_suffix(".tgz")?;
    if is_safe_version(version) {
        Some(version.to_string())
    } else {
        None
    }
}

fn parse_desktop_release_notes(html: &str) -> Vec<DockerOfflinePackage> {
    let mut packages = Vec::new();
    let mut seen = HashSet::new();
    let mut versions = 0usize;

    for section in html.split("<h2 ").skip(1) {
        if versions >= DESKTOP_VERSION_LIMIT {
            break;
        }
        let Some(version) = desktop_version(section) else {
            continue;
        };
        let windows = desktop_build(section, "https://desktop.docker.com/win/main/amd64/");
        let mac_arm = desktop_build(section, "https://desktop.docker.com/mac/main/arm64/");
        let mac_intel = desktop_build(section, "https://desktop.docker.com/mac/main/amd64/");
        if windows.is_none() && mac_arm.is_none() && mac_intel.is_none() {
            continue;
        }
        versions += 1;

        if let Some(build) = windows {
            push_desktop_package(
                &mut packages,
                &mut seen,
                "windows",
                "amd64",
                &version,
                &build,
            );
        }
        if let Some(build) = mac_arm {
            push_desktop_package(&mut packages, &mut seen, "macos", "arm64", &version, &build);
        }
        if let Some(build) = mac_intel {
            push_desktop_package(&mut packages, &mut seen, "macos", "amd64", &version, &build);
        }
    }
    packages
}

fn desktop_version(section: &str) -> Option<String> {
    let marker = "href=#";
    let start = section.find(marker)? + marker.len();
    let text_start = section[start..].find('>')? + start + 1;
    let text_end = section[text_start..].find("</a>")? + text_start;
    let version = section[text_start..text_end].trim();
    if is_safe_version(version) && version.contains('.') {
        Some(version.to_string())
    } else {
        None
    }
}

fn desktop_build(section: &str, prefix: &str) -> Option<String> {
    let start = section.find(prefix)? + prefix.len();
    let build: String = section[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    if build.is_empty() {
        None
    } else {
        Some(build)
    }
}

fn push_desktop_package(
    packages: &mut Vec<DockerOfflinePackage>,
    seen: &mut HashSet<(String, String, String, String)>,
    platform: &str,
    arch: &str,
    version: &str,
    build: &str,
) {
    let key = (
        platform.to_string(),
        arch.to_string(),
        version.to_string(),
        build.to_string(),
    );
    if !seen.insert(key) {
        return;
    }
    let file_name = if platform == "windows" {
        format!("Docker-Desktop-{version}-{build}-{arch}.exe")
    } else {
        format!("Docker-{version}-{build}-{arch}.dmg")
    };
    packages.push(DockerOfflinePackage {
        platform: platform.to_string(),
        arch: arch.to_string(),
        version: version.to_string(),
        build: build.to_string(),
        file_name,
    });
}

fn normalize_packages(packages: &mut Vec<DockerOfflinePackage>) {
    packages.sort_by(|left, right| {
        platform_rank(left)
            .cmp(&platform_rank(right))
            .then_with(|| compare_versions(&right.version, &left.version))
            .then_with(|| right.build.cmp(&left.build))
    });
    packages.dedup_by(|left, right| {
        left.platform == right.platform
            && left.arch == right.arch
            && left.version == right.version
            && left.build == right.build
    });
}

fn platform_rank(package: &DockerOfflinePackage) -> (u8, &str) {
    let rank = match (package.platform.as_str(), package.arch.as_str()) {
        ("linux", "x86_64") => 0,
        ("linux", "aarch64") => 1,
        ("windows", "amd64") => 2,
        ("macos", "arm64") => 3,
        ("macos", "amd64") => 4,
        _ => 5,
    };
    (rank, package.platform.as_str())
}

fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let left_parts = version_parts(left);
    let right_parts = version_parts(right);
    for index in 0..left_parts.len().max(right_parts.len()) {
        let a = left_parts.get(index).copied().unwrap_or(0);
        let b = right_parts.get(index).copied().unwrap_or(0);
        match a.cmp(&b) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    left.cmp(right)
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn default_platform(packages: &[DockerOfflinePackage]) -> String {
    let preferred = match std::env::consts::OS {
        "windows" => "windows:amd64",
        "macos" if std::env::consts::ARCH == "aarch64" => "macos:arm64",
        "macos" => "macos:amd64",
        "linux" if std::env::consts::ARCH == "aarch64" => "linux:aarch64",
        _ => "linux:x86_64",
    };
    if packages
        .iter()
        .any(|package| platform_key(package) == preferred)
    {
        preferred.to_string()
    } else {
        packages
            .first()
            .map(platform_key)
            .unwrap_or_else(|| "linux:x86_64".to_string())
    }
}

fn platform_key(package: &DockerOfflinePackage) -> String {
    format!("{}:{}", package.platform, package.arch)
}

fn download_dir() -> String {
    dirs::download_dir()
        .or_else(dirs::document_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
        .to_string_lossy()
        .to_string()
}

fn is_safe_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_+".contains(character))
        && version
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
}

fn is_safe_build(build: &str) -> bool {
    !build.is_empty()
        && build.len() <= 16
        && build.chars().all(|character| character.is_ascii_digit())
}

fn package_download_url(
    platform: &str,
    arch: &str,
    version: &str,
    build: &str,
) -> Result<String, String> {
    if !is_safe_version(version) {
        return Err("所选 Docker 版本无效，请重新选择".to_string());
    }
    match (platform, arch) {
        ("linux", "x86_64") | ("linux", "aarch64") => Ok(format!(
            "https://download.docker.com/linux/static/stable/{arch}/docker-{version}.tgz"
        )),
        ("windows", "amd64") if is_safe_build(build) => Ok(format!(
            "https://desktop.docker.com/win/main/amd64/{build}/Docker%20Desktop%20Installer.exe"
        )),
        ("macos", "arm64" | "amd64") if is_safe_build(build) => Ok(format!(
            "https://desktop.docker.com/mac/main/{arch}/{build}/Docker.dmg"
        )),
        _ => Err("所选 Docker 平台或版本无效，请重新选择".to_string()),
    }
}

fn package_file_name(
    platform: &str,
    arch: &str,
    version: &str,
    build: &str,
) -> Result<String, String> {
    if !is_safe_version(version) {
        return Err("所选 Docker 版本无效，请重新选择".to_string());
    }
    match (platform, arch) {
        ("linux", "x86_64") | ("linux", "aarch64") => Ok(format!("docker-{version}.tgz")),
        ("windows", "amd64") if is_safe_build(build) => {
            Ok(format!("Docker-Desktop-{version}-{build}-amd64.exe"))
        }
        ("macos", "arm64" | "amd64") if is_safe_build(build) => {
            Ok(format!("Docker-{version}-{build}-{arch}.dmg"))
        }
        _ => Err("所选 Docker 平台或版本无效，请重新选择".to_string()),
    }
}

fn resolve_target(save_path: &str, file_name: &str) -> Result<PathBuf, String> {
    let raw = save_path.trim();
    if raw.is_empty() {
        return Err("请选择离线包的保存位置".to_string());
    }
    let selected = PathBuf::from(raw);
    if !selected.is_absolute() {
        return Err("请选择完整的保存位置".to_string());
    }
    let target = if selected.is_dir() {
        selected.join(file_name)
    } else {
        selected
    };
    if target.file_name().is_none() {
        return Err("请选择离线包的文件名".to_string());
    }
    Ok(target)
}

fn download_docker_offline_package_blocking(
    platform: String,
    arch: String,
    version: String,
    build: String,
    save_path: String,
    on_event: Channel<DockerDownloadProgress>,
) -> Result<DockerDownloadResult, String> {
    let url = package_download_url(&platform, &arch, &version, &build)?;
    let file_name = package_file_name(&platform, &arch, &version, &build)?;
    let target = resolve_target(&save_path, &file_name)?;
    let parent = target
        .parent()
        .ok_or_else(|| "所选保存位置无效，请重新选择".to_string())?;
    fs::create_dir_all(parent).map_err(|error| {
        log::error!(target: "docker-download", "创建保存目录失败：{error}");
        "无法创建保存目录，请检查目录权限".to_string()
    })?;
    let staged = parent.join(format!(
        ".{file_name}.{}.part",
        crate::core::backup::unique_suffix()
    ));
    let previous = parent.join(format!(
        ".{file_name}.{}.previous",
        crate::core::backup::unique_suffix()
    ));

    log::info!(
        target: "docker-download",
        "开始下载 Docker 离线包：平台={platform} 架构={arch} 版本={version} 构建={build} 地址={}",
        crate::logging::safe_url(&url)
    );

    let result = (|| -> Result<DockerDownloadResult, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(1800))
            .user_agent(concat!("deploy-desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| {
                log::error!(target: "docker-download", "创建下载客户端失败：{error}");
                "无法连接 Docker 官方下载站，请稍后重试".to_string()
            })?;
        let mut response = client.get(&url).send().map_err(|error| {
            log::warn!(target: "docker-download", "下载请求失败：{error}");
            "无法连接 Docker 官方下载站，请检查网络后重试".to_string()
        })?;
        if !response.status().is_success() {
            log::warn!(
                target: "docker-download",
                "下载返回异常状态：{}",
                response.status()
            );
            return Err(format!(
                "Docker 官方下载站返回 HTTP {}，请更换版本或稍后重试",
                response.status()
            ));
        }
        let total = response.content_length().unwrap_or(0);
        if total > MAX_DOWNLOAD_BYTES {
            return Err("离线包体积超过 4 GB，已停止下载".to_string());
        }
        let _ = on_event.send(DockerDownloadProgress {
            stage: "download".to_string(),
            downloaded: 0,
            total,
        });

        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(|error| {
                log::error!(target: "docker-download", "创建临时文件失败：{error}");
                "无法写入所选位置，请检查目录权限和剩余空间".to_string()
            })?;
        let mut buffer = vec![0u8; 256 * 1024];
        let mut downloaded = 0u64;
        let mut hasher = Sha256::new();
        loop {
            let read = response.read(&mut buffer).map_err(|error| {
                log::warn!(target: "docker-download", "读取下载数据失败：{error}");
                "下载中断，请检查网络后重试".to_string()
            })?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read]).map_err(|error| {
                log::error!(target: "docker-download", "写入离线包失败：{error}");
                "无法写入所选位置，请检查目录权限和剩余空间".to_string()
            })?;
            hasher.update(&buffer[..read]);
            downloaded += read as u64;
            if downloaded > MAX_DOWNLOAD_BYTES {
                return Err("离线包体积超过 4 GB，已停止下载".to_string());
            }
            let _ = on_event.send(DockerDownloadProgress {
                stage: "download".to_string(),
                downloaded,
                total,
            });
        }
        output.flush().map_err(|error| {
            log::error!(target: "docker-download", "刷新离线包写入失败：{error}");
            "无法完成文件写入，请检查磁盘空间".to_string()
        })?;
        output.sync_all().map_err(|error| {
            log::error!(target: "docker-download", "同步离线包失败：{error}");
            "无法完成文件写入，请检查磁盘空间".to_string()
        })?;
        drop(output);

        if downloaded == 0 {
            return Err("下载内容为空，请稍后重试".to_string());
        }
        if total > 0 && downloaded != total {
            return Err("下载内容不完整，请检查网络后重试".to_string());
        }

        let had_previous =
            crate::core::backup::swap_staged(&staged, &target, &previous).map_err(|failure| {
                log::error!(
                    target: "docker-download",
                    "保存离线包失败：阶段={:?} 错误={}",
                    failure.stage,
                    failure.error
                );
                if failure.rollback.is_err() {
                    log::error!(target: "docker-download", "保存失败后恢复原文件失败");
                }
                "无法保存离线包，请检查目标位置是否可写".to_string()
            })?;
        let _ = crate::core::backup::discard_previous(&previous, had_previous);

        let sha256 = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = target.to_string_lossy().to_string();
        log::info!(
            target: "docker-download",
            "Docker 离线包下载完成：平台={platform} 版本={version} 大小={downloaded} 字节 路径={path}"
        );
        let _ = on_event.send(DockerDownloadProgress {
            stage: "done".to_string(),
            downloaded,
            total: downloaded,
        });
        Ok(DockerDownloadResult {
            path,
            size_bytes: downloaded,
            sha256,
        })
    })();

    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX_INDEX: &str = r#"
<a href="docker-29.9.2.tgz">docker-29.9.2.tgz</a>
<a href="docker-29.9.3.tgz">docker-29.9.3.tgz</a>
<a href="../">../</a>
"#;

    const DESKTOP_NOTES: &str = r#"
<h2 class="release" id=4980><a href=#4980>4.98.0</a></h2><em>2026-11-03</em>
<a href="https://desktop.docker.com/win/main/amd64/245422/Docker%20Desktop%20Installer.exe?utm_source=docker">Windows</a>
<a href="https://desktop.docker.com/mac/main/arm64/245422/Docker.dmg">Mac with Apple chip</a>
<a href="https://desktop.docker.com/mac/main/amd64/245422/Docker.dmg">Mac with Intel chip</a>
<h2 class="release" id=4970><a href=#4970>4.97.0</a></h2><em>2026-10-26</em>
<a href="https://desktop.docker.com/win/main/amd64/244584/Docker%20Desktop%20Installer.exe">Windows</a>
<a href="https://desktop.docker.com/mac/main/arm64/244584/Docker.dmg">Mac with Apple chip</a>
"#;

    #[test]
    fn parses_linux_static_versions() {
        let packages = parse_linux_index(LINUX_INDEX, "x86_64");
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].version, "29.9.2");
        assert_eq!(packages[1].file_name, "docker-29.9.3.tgz");
    }

    #[test]
    fn parses_desktop_versions_and_architectures() {
        let packages = parse_desktop_release_notes(DESKTOP_NOTES);
        assert_eq!(packages.len(), 5);
        assert!(packages.iter().any(|package| {
            package.platform == "windows"
                && package.arch == "amd64"
                && package.version == "4.98.0"
                && package.build == "245422"
        }));
        assert!(packages.iter().any(|package| {
            package.platform == "macos" && package.arch == "amd64" && package.version == "4.98.0"
        }));
    }

    #[test]
    fn builds_only_official_download_urls() {
        assert_eq!(
            package_download_url("linux", "x86_64", "29.9.3", "").unwrap(),
            "https://download.docker.com/linux/static/stable/x86_64/docker-29.9.3.tgz"
        );
        assert_eq!(
            package_download_url("windows", "amd64", "4.98.0", "245422").unwrap(),
            "https://desktop.docker.com/win/main/amd64/245422/Docker%20Desktop%20Installer.exe"
        );
        assert!(package_download_url("linux", "x86_64", "../../etc/passwd", "").is_err());
        assert!(package_download_url("windows", "amd64", "4.98.0", "245422/../../x").is_err());
    }

    #[test]
    fn sorts_versions_numerically() {
        let mut packages = vec![
            DockerOfflinePackage {
                platform: "linux".to_string(),
                arch: "x86_64".to_string(),
                version: "29.8.0".to_string(),
                build: String::new(),
                file_name: "docker-29.8.0.tgz".to_string(),
            },
            DockerOfflinePackage {
                platform: "linux".to_string(),
                arch: "x86_64".to_string(),
                version: "29.10.0".to_string(),
                build: String::new(),
                file_name: "docker-29.10.0.tgz".to_string(),
            },
        ];
        normalize_packages(&mut packages);
        assert_eq!(packages[0].version, "29.10.0");
    }
}
