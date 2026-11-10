use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::core::process::{self, Input, Options};

/// 远程脚本共用的 Docker 访问入口：统一处理权限探测与免密 sudo 降级。
/// 静态脚本，不拼接任何外部输入；由 `remote_ssh::client` 注入到 SSH 脚本。
pub(crate) const REMOTE_DOCKER_PREAMBLE: &str = include_str!("../../../shell/docker.sh");

#[derive(Debug, Serialize)]
pub struct DockerInstallation {
    pub os: String,
    pub installed: bool,
    pub version: Option<String>,
}

#[tauri::command]
pub async fn check_docker_installation() -> Result<DockerInstallation, String> {
    super::blocking::run(move || {
        let os = std::env::consts::OS;
        let (_, markers) = current_docker_locations();
        inspect_installation(os, find_docker_binary().as_deref(), &markers)
    })
    .await
}

#[tauri::command]
pub async fn open_docker_installation() -> Result<(), String> {
    super::blocking::run(move || {
        let url = installation_url(std::env::consts::OS)?;
        #[cfg(target_os = "macos")]
        let mut command = Command::new("open");
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = Command::new("rundll32.exe");
            command.arg("url.dll,FileProtocolHandler");
            command
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut command = Command::new("xdg-open");
        command.arg(url);
        let output = run_docker_check(command, Duration::from_secs(10))
            .map_err(|_| "无法打开 Docker 官方安装页面，请检查默认浏览器后重试。".to_string())?;
        if !output.status.success() {
            return Err("无法打开 Docker 官方安装页面，请检查默认浏览器后重试。".to_string());
        }
        Ok(())
    })
    .await
}

fn installation_url(os: &str) -> Result<&'static str, String> {
    match os {
        "windows" => Ok("https://docs.docker.com/desktop/setup/install/windows-install/"),
        "macos" => Ok("https://docs.docker.com/desktop/setup/install/mac-install/"),
        "linux" => Ok("https://docs.docker.com/engine/install/"),
        _ => Err("当前系统暂不支持安装引导，请前往 Docker 官网查看安装要求。".to_string()),
    }
}

fn docker_locations(
    os: &str,
    home: Option<&Path>,
    program_files: Option<&Path>,
    local_app_data: Option<&Path>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut binaries = Vec::new();
    let mut markers = Vec::new();
    if os == "windows" {
        let mut roots = Vec::new();
        if let Some(root) = program_files {
            roots.push(root.join("Docker/Docker"));
        }
        if let Some(root) = local_app_data {
            roots.push(root.join("Programs/DockerDesktop"));
            roots.push(root.join("Programs/Docker/Docker"));
            roots.push(root.join("Docker"));
        }
        for root in roots {
            binaries.push(root.join("resources/bin/docker.exe"));
            markers.push(root.join("Docker Desktop.exe"));
        }
    } else {
        for directory in ["/usr/bin", "/bin", "/usr/local/bin", "/opt/homebrew/bin"] {
            binaries.push(Path::new(directory).join("docker"));
        }
        if let Some(home) = home {
            binaries.push(home.join(".docker/bin/docker"));
        }
        if os == "macos" {
            let mut roots = vec![PathBuf::from("/Applications/Docker.app")];
            if let Some(home) = home {
                roots.push(home.join("Applications/Docker.app"));
                binaries.push(home.join(".orbstack/bin/docker"));
                markers.push(home.join("Applications/OrbStack.app/Contents/Info.plist"));
            }
            for root in roots {
                binaries.push(root.join("Contents/Resources/bin/docker"));
                markers.push(root.join("Contents/Info.plist"));
            }
            binaries.push(PathBuf::from(
                "/Applications/OrbStack.app/Contents/MacOS/xbin/docker",
            ));
            markers.push(PathBuf::from(
                "/Applications/OrbStack.app/Contents/Info.plist",
            ));
        } else if os == "linux" {
            binaries.push(PathBuf::from("/opt/docker-desktop/bin/docker"));
            markers.push(PathBuf::from("/opt/docker-desktop/bin/com.docker.backend"));
        }
    }
    (binaries, markers)
}

fn current_docker_locations() -> (Vec<PathBuf>, Vec<PathBuf>) {
    docker_locations(
        std::env::consts::OS,
        dirs::home_dir().as_deref(),
        std::env::var_os("ProgramW6432")
            .or_else(|| std::env::var_os("ProgramFiles"))
            .map(PathBuf::from)
            .as_deref(),
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .as_deref(),
    )
}

pub(crate) fn find_docker_binary() -> Option<PathBuf> {
    let (mut binaries, _) = current_docker_locations();
    if let Some(path) = std::env::var_os("PATH") {
        let name = if cfg!(windows) {
            "docker.exe"
        } else {
            "docker"
        };
        binaries.extend(std::env::split_paths(&path).map(|directory| directory.join(name)));
    }
    binaries.into_iter().find(|path| path.is_file())
}

/// Docker 桌面版把凭据助手放在应用资源目录或用户目录中；
/// GUI 进程继承到的 PATH 往往不含这些目录，导致 `docker pull` 找不到
/// `docker-credential-*`。子进程统一使用补全后的 PATH。
fn docker_path_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(binary) = find_docker_binary() {
        if let Some(parent) = binary.parent() {
            paths.push(parent.to_path_buf());
        }
    }
    for directory in [
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/usr/bin",
        "/bin",
        "/snap/bin",
        "/opt/docker-desktop/bin",
    ] {
        paths.push(PathBuf::from(directory));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".docker/bin"));
        paths.push(home.join(".orbstack/bin"));
        #[cfg(target_os = "macos")]
        {
            paths.push(home.join("Applications/Docker.app/Contents/Resources/bin"));
            paths.push(home.join("Applications/OrbStack.app/Contents/MacOS/xbin"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from(
            "/Applications/Docker.app/Contents/Resources/bin",
        ));
        paths.push(PathBuf::from(
            "/Applications/OrbStack.app/Contents/MacOS/xbin",
        ));
    }
    #[cfg(target_os = "windows")]
    {
        for variable in ["ProgramW6432", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(variable) {
                let root = PathBuf::from(root);
                paths.push(root.join("Docker/Docker/resources/bin"));
                paths.push(root.join("Programs/Docker/Docker/resources/bin"));
                paths.push(root.join("Programs/DockerDesktop/resources/bin"));
            }
        }
        if let Some(home) = dirs::home_dir() {
            paths.push(home.join(".docker/bin"));
        }
    }
    paths.extend(
        std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| path.is_dir() && seen.insert(path.clone()));
    paths
}

pub(crate) fn docker_path_env() -> Option<OsString> {
    let paths = docker_path_candidates();
    if paths.is_empty() {
        None
    } else {
        std::env::join_paths(paths).ok()
    }
}

pub(crate) fn docker_command() -> Command {
    let binary = find_docker_binary().unwrap_or_else(|| {
        PathBuf::from(if cfg!(windows) {
            "docker.exe"
        } else {
            "docker"
        })
    });
    let mut command = Command::new(binary);
    if let Some(path) = docker_path_env() {
        command.env("PATH", path);
    }
    command
}

fn compose_binary_candidates() -> Vec<PathBuf> {
    let name = if cfg!(windows) {
        "docker-compose.exe"
    } else {
        "docker-compose"
    };
    let mut candidates = Vec::new();
    if let Some(directory) = find_docker_binary()
        .and_then(|binary| binary.parent().map(Path::to_path_buf))
    {
        candidates.push(directory.join(name));
    }
    for directory in ["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin", "/bin"] {
        candidates.push(Path::new(directory).join(name));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|directory| directory.join(name)));
    }
    candidates
}

/// 独立的 `docker-compose` 二进制（旧版 Compose）。优先使用 `docker compose` 插件，
/// 这里只在插件不可用时作为回退入口，同样补齐凭据助手所需的 PATH。
pub(crate) fn docker_compose_command() -> Command {
    let binary = compose_binary_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(windows) {
                "docker-compose.exe"
            } else {
                "docker-compose"
            })
        });
    let mut command = Command::new(binary);
    if let Some(path) = docker_path_env() {
        command.env("PATH", path);
    }
    command
}

/// 判断错误输出是否为 Docker socket 权限不足，避免把文件权限问题误判为 Docker 权限。
pub(crate) fn docker_socket_permission_denied(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    (lower.contains("permission denied")
        && (lower.contains("docker.sock")
            || lower.contains("docker daemon")
            || lower.contains("docker_engine")))
        || (lower.contains("access is denied") && lower.contains("docker_engine"))
}

/// 把 Docker CLI 的常见失败翻译成用户可执行的提示；未识别时返回 None。
pub(crate) fn docker_failure_hint(stderr: &str) -> Option<&'static str> {
    if docker_socket_permission_denied(stderr) {
        return Some(
            "当前用户无权访问 Docker。请把用户加入 docker 用户组后重新登录，或在系统设置中允许本应用访问 Docker。",
        );
    }
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("cannot connect to the docker daemon")
        || lower.contains("is the docker daemon running")
        || lower.contains("error during connect")
    {
        return Some("无法连接 Docker 服务，请先启动 Docker Desktop 或 Docker 服务后重试。");
    }
    if lower.contains("docker: command not found")
        || lower.contains("docker: not found")
        || (lower.contains("no such file or directory") && lower.contains("docker"))
    {
        return Some("未找到 Docker 命令，请先安装 Docker 并确认已加入 PATH。");
    }
    None
}

fn inspect_installation(
    os: &str,
    binary: Option<&Path>,
    markers: &[PathBuf],
) -> Result<DockerInstallation, String> {
    let mut installation = DockerInstallation {
        os: os.to_string(),
        installed: markers.iter().any(|path| path.is_file()),
        version: None,
    };
    if let Some(binary) = binary {
        let mut command = Command::new(binary);
        command.arg("--version");
        let output = run_docker_check(command, Duration::from_secs(5)).map_err(|_| {
            "无法确认 Docker 安装状态，请检查 Docker 是否可运行后重新检测。".to_string()
        })?;
        if !output.status.success() {
            return Err(
                "检测到 Docker，但无法读取版本。请检查 Docker 安装后重新检测。".to_string(),
            );
        }
        installation.installed = true;
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        installation.version = if version.is_empty() {
            None
        } else {
            Some(version)
        };
    }
    Ok(installation)
}

fn run_docker_check(command: Command, timeout: Duration) -> Result<std::process::Output, String> {
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        let mut command = command;
        command.creation_flags(0x08000000);
        command
    };
    process::run(
        command,
        Input::Bytes(Vec::new()),
        Options {
            timeout,
            max_output: 8192,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn docker_path_keeps_helper_directories_and_existing_entries() {
        let path = docker_path_env().expect("PATH should be available");
        let directories: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert!(directories.iter().any(|directory| {
            directory == Path::new("/usr/local/bin")
                || directory == Path::new("/opt/homebrew/bin")
                || directory == Path::new("/usr/bin")
        }));
        for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .filter(|directory| directory.is_dir())
        {
            assert!(directories.contains(&directory));
        }
    }

    #[test]
    fn installation_guides_match_the_operating_system() {
        assert!(installation_url("windows")
            .unwrap()
            .ends_with("/windows-install/"));
        assert!(installation_url("macos")
            .unwrap()
            .ends_with("/mac-install/"));
        assert_eq!(
            installation_url("linux").unwrap(),
            "https://docs.docker.com/engine/install/"
        );
        assert!(installation_url("unsupported").is_err());
    }

    #[test]
    fn desktop_locations_cover_system_and_user_installations_without_path() {
        let home = Path::new("user");
        let program_files = Path::new("program-files");
        let local_app_data = Path::new("local-app-data");
        let (binaries, markers) = docker_locations(
            "windows",
            Some(home),
            Some(program_files),
            Some(local_app_data),
        );
        assert!(binaries.contains(&program_files.join("Docker/Docker/resources/bin/docker.exe")));
        assert!(binaries
            .contains(&local_app_data.join("Programs/DockerDesktop/resources/bin/docker.exe")));
        assert!(markers.contains(&program_files.join("Docker/Docker/Docker Desktop.exe")));
        let (binaries, markers) = docker_locations("macos", Some(home), None, None);
        assert!(binaries.contains(&PathBuf::from(
            "/Applications/Docker.app/Contents/Resources/bin/docker"
        )));
        assert!(binaries.contains(&home.join(".docker/bin/docker")));
        assert!(binaries.contains(&home.join(".orbstack/bin/docker")));
        assert!(binaries.contains(&PathBuf::from(
            "/Applications/OrbStack.app/Contents/MacOS/xbin/docker"
        )));
        assert!(markers.contains(&home.join("Applications/Docker.app/Contents/Info.plist")));
        assert!(markers.contains(&home.join("Applications/OrbStack.app/Contents/Info.plist")));
        let (binaries, _) = docker_locations("linux", Some(home), None, None);
        assert!(binaries.contains(&PathBuf::from("/usr/bin/docker")));
        assert!(binaries.contains(&PathBuf::from("/opt/docker-desktop/bin/docker")));
    }

    #[test]
    fn missing_docker_is_reported_on_each_platform() {
        for os in ["windows", "linux", "macos"] {
            let installation = inspect_installation(os, None, &[]).unwrap();
            assert_eq!(installation.os, os);
            assert!(!installation.installed);
            assert!(installation.version.is_none());
        }
    }

    #[test]
    fn docker_failure_hint_maps_permission_and_daemon_errors() {
        assert!(docker_failure_hint(
            "permission denied while trying to connect to the Docker daemon socket"
        )
        .unwrap()
        .contains("docker 用户组"));
        assert!(docker_failure_hint("Cannot connect to the Docker daemon at unix:///var/run/docker.sock")
            .unwrap()
            .contains("启动 Docker"));
        assert!(docker_failure_hint("some unrelated error").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn remote_preamble_prefers_direct_docker_access() {
        let directory = mock_runner_dir("direct");
        write_script(&directory.join("docker"), "echo \"RUN:$*\"");
        let output = run_remote_preamble(&directory, "docker ps");
        std::fs::remove_dir_all(&directory).ok();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("RUN:ps"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_preamble_falls_back_to_passwordless_sudo() {
        let directory = mock_runner_dir("sudo");
        write_script(
            &directory.join("docker"),
            "if [ \"$1\" = info ] && [ \"${DEPLOY_TEST_ROOT:-0}\" != 1 ]; then\n  echo 'permission denied while trying to connect to the Docker daemon socket' >&2\n  exit 1\nfi\necho \"RUN:$*\"",
        );
        write_script(
            &directory.join("sudo"),
            "while [ \"$1\" = -n ] || [ \"$1\" = -E ]; do shift; done\nexport DEPLOY_TEST_ROOT=1\nexec \"$@\"",
        );
        let output = run_remote_preamble_with_fallback_path(&directory, "docker ps");
        std::fs::remove_dir_all(&directory).ok();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("RUN:ps"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_preamble_does_not_fall_back_when_daemon_is_unreachable() {
        let directory = mock_runner_dir("unreachable");
        write_script(
            &directory.join("docker"),
            "echo 'Cannot connect to the Docker daemon at unix:///var/run/docker.sock' >&2\nexit 1",
        );
        write_script(&directory.join("sudo"), "echo SUDO-CALLED\nexit 0");
        let output = run_remote_preamble_with_fallback_path(&directory, "docker ps");
        std::fs::remove_dir_all(&directory).ok();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("SUDO-CALLED"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Docker 权限不足"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_preamble_passes_docker_config_to_sudo() {
        let directory = mock_runner_dir("sudo-env");
        write_script(
            &directory.join("docker"),
            "if [ \"$1\" = info ] && [ \"${DEPLOY_TEST_ROOT:-0}\" != 1 ]; then\n  echo 'permission denied while trying to connect to the Docker daemon socket' >&2\n  exit 1\nfi\necho \"RUN:$*\"",
        );
        write_script(
            &directory.join("sudo"),
            "while [ \"$1\" = -n ] || [ \"$1\" = -E ]; do shift; done\necho \"SUDO:$*\"\nexport DEPLOY_TEST_ROOT=1\nexec \"$@\"",
        );
        let output = run_remote_preamble_with_fallback_path(
            &directory,
            "DOCKER_CONFIG=/tmp/deploy-test\nexport DOCKER_CONFIG\ndocker ps",
        );
        std::fs::remove_dir_all(&directory).ok();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("DOCKER_CONFIG=/tmp/deploy-test"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_preamble_explains_missing_socket_permission() {
        let directory = mock_runner_dir("denied");
        write_script(
            &directory.join("docker"),
            "echo 'permission denied while trying to connect to the Docker daemon socket' >&2\nexit 1",
        );
        write_script(&directory.join("sudo"), "exit 1");
        let output = run_remote_preamble_with_fallback_path(&directory, "docker ps");
        std::fs::remove_dir_all(&directory).ok();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Docker 权限不足"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_preamble_keeps_version_working_without_socket_permission() {
        let directory = mock_runner_dir("version");
        write_script(
            &directory.join("docker"),
            "if [ \"$1\" = --version ]; then\n  echo 'Docker version 27.0.0'\n  exit 0\nfi\necho 'permission denied while trying to connect to the Docker daemon socket' >&2\nexit 1",
        );
        write_script(&directory.join("sudo"), "exit 1");
        let output = run_remote_preamble_with_fallback_path(&directory, "docker --version");
        std::fs::remove_dir_all(&directory).ok();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Docker version 27.0.0"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Docker 权限不足"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_preamble_does_not_shadow_missing_docker() {
        let directory = mock_runner_dir("missing");
        let output = run_remote_preamble(
            &directory,
            "if command -v docker >/dev/null 2>&1; then echo FOUND; else echo MISSING; fi",
        );
        std::fs::remove_dir_all(&directory).ok();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("MISSING"));
    }

    #[test]
    fn installed_desktop_is_not_missing_when_cli_setup_is_incomplete() {
        let marker = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(
            inspect_installation("macos", None, &[marker])
                .unwrap()
                .installed
        );
    }

    #[cfg(unix)]
    #[test]
    fn installation_check_does_not_require_a_running_daemon() {
        let binary = mock_docker(
            "if [ \"$1\" = --version ]; then echo 'Docker version 27.0.0'; else exit 29; fi",
        );
        let installation = inspect_installation("linux", Some(&binary), &[]).unwrap();
        std::fs::remove_dir_all(binary.parent().unwrap()).unwrap();
        assert!(installation.installed);
        assert_eq!(
            installation.version.as_deref(),
            Some("Docker version 27.0.0")
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_version_check_is_not_reported_as_missing_docker() {
        let binary = mock_docker("exit 1");
        let result = inspect_installation("linux", Some(&binary), &[]);
        std::fs::remove_dir_all(binary.parent().unwrap()).unwrap();
        assert!(result.unwrap_err().contains("无法读取版本"));
    }

    #[cfg(unix)]
    fn mock_runner_dir(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "docker-runner-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    fn run_remote_preamble(path: &Path, script: &str) -> std::process::Output {
        run_remote_preamble_with_path(path, script, false)
    }

    #[cfg(unix)]
    fn run_remote_preamble_with_fallback_path(path: &Path, script: &str) -> std::process::Output {
        run_remote_preamble_with_path(path, script, true)
    }

    #[cfg(unix)]
    fn run_remote_preamble_with_path(
        path: &Path,
        script: &str,
        keep_system_path: bool,
    ) -> std::process::Output {
        let search_path = if keep_system_path {
            std::env::join_paths(
                std::iter::once(path.to_path_buf()).chain(
                    std::env::var_os("PATH")
                        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
                        .unwrap_or_default(),
                ),
            )
            .unwrap()
        } else {
            path.as_os_str().to_owned()
        };
        let mut command = Command::new("/bin/bash");
        command.args(["--noprofile", "--norc", "-s", "--"]);
        command.env("PATH", search_path);
        process::run(
            command,
            Input::Bytes(format!("{REMOTE_DOCKER_PREAMBLE}\n{script}\n").into_bytes()),
            Options {
                timeout: Duration::from_secs(20),
                max_output: 64 * 1024,
            },
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn mock_docker(script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "docker-check-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let binary = directory.join("docker");
        std::fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        binary
    }
}
