use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::core::process::{self, Input, Options};

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
            }
            for root in roots {
                binaries.push(root.join("Contents/Resources/bin/docker"));
                markers.push(root.join("Contents/Info.plist"));
            }
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
        assert!(markers.contains(&home.join("Applications/Docker.app/Contents/Info.plist")));
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
