//! 远程主机命令：连通性测试、免密登录初始化、环境检查与部署执行、远程 Docker 与数据库操作。
//!
//! 子模块分工：`client` 提供 SSH 命令拼装与脚本执行，`auth` 负责密钥初始化，
//! `env` 负责环境检查与部署执行，`docker` 与 `database` 分别封装远程 Docker 和
//! OceanBase 操作。命令名、参数与返回结构保持不变。

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use tauri::Emitter;

use super::db_ops::{DbImportResult, OceanBaseHealth};
use super::docker_engine::{DockerInstallResult, DockerInstallStep};
use super::docker_ops::{ComposeActionResult, ImageLoadProgress};

mod auth;
mod client;
mod database;
mod docker;
mod env;
mod model;

pub(crate) use auth::*;
pub(crate) use client::*;
pub(crate) use database::*;
pub(crate) use docker::*;
pub(crate) use env::*;
pub use model::*;

#[tauri::command]
pub async fn test_ssh_connection(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<SshTestResult, String> {
    let target = format!("{user}@{host}:{port}");
    let result = crate::commands::blocking::run(move || {
        test_ssh_connection_blocking(host, port, user, key_path)
    })
    .await;
    match &result {
        Ok(info) if info.success => {
            log::info!(target: "ssh", "SSH 连接成功：{target} 往返={:?}ms", info.latency_ms)
        }
        Ok(info) => log::warn!(target: "ssh", "SSH 连接未通过：{target}：{}", info.message),
        Err(error) => log::error!(target: "ssh", "SSH 连接失败：{target}：{error}"),
    }
    result
}

#[tauri::command]
pub async fn setup_remote_ssh_key(
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    key_path: Option<String>,
) -> Result<SshTestResult, String> {
    crate::commands::blocking::run(move || {
        setup_remote_ssh_key_blocking(host, port, user, password, key_path)
    })
    .await
}

#[tauri::command]
pub async fn check_remote_env(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<RemoteEnvResult, String> {
    let target = format!("{user}@{host}:{port}");
    let result = crate::commands::blocking::run(move || {
        check_remote_env_blocking(host, port, user, key_path)
    })
    .await;
    match &result {
        Ok(info) if info.is_ready => log::info!(
            target: "env",
            "目标机环境检查通过：{target}，待处理 {} 项",
            info.issues.len()
        ),
        Ok(info) => log::warn!(
            target: "env",
            "目标机环境未就绪：{target}，待处理 {} 项：{}",
            info.issues.len(),
            info.issues.join("；")
        ),
        Err(error) => log::error!(target: "env", "目标机环境检查失败：{target}：{error}"),
    }
    result
}

#[tauri::command]
pub async fn run_remote_deploy_action(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    mode: String, // "deploy-all", "server-only", "gateway-app-only", "frontend-only", "status", "start", "stop", "restart"
    target_service: Option<String>,
    deploy_dir: Option<String>,
) -> Result<SshDeployExecutionResult, String> {
    crate::commands::blocking::run(move || {
        run_remote_deploy_action_blocking(
            host,
            port,
            user,
            key_path,
            mode,
            target_service,
            deploy_dir,
        )
    })
    .await
}

#[tauri::command]
pub async fn install_remote_docker_engine(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
) -> Result<DockerInstallResult, String> {
    crate::commands::blocking::run(move || {
        install_remote_docker_engine_blocking(host, port, user, key_path, bundle_dir)
    })
    .await
}

#[tauri::command]
pub async fn load_remote_images(
    window: tauri::Window,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
) -> Result<Vec<String>, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let images_dir = format!("{}/images", bundle_dir.trim_end_matches('/'));
    let list_script = format!(
        "set -e\nIMAGES={}\n[ -d \"$IMAGES\" ] || {{ echo \"镜像目录不存在: $IMAGES\" >&2; exit 3; }}\nfind \"$IMAGES\" -maxdepth 1 -type f \\( -name '*.tar' -o -name '*.tgz' -o -name '*.tar.gz' \\) | sort\n",
        shell_quote(&images_dir)
    );
    let list_out = run_ssh_script(&host, port, &user, key.as_deref(), &list_script)?;
    if !list_out.status.success() {
        let stderr = String::from_utf8_lossy(&list_out.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key.as_deref()));
    }

    let files: Vec<String> = String::from_utf8_lossy(&list_out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    if files.is_empty() {
        return Err(format!(
            "远程镜像目录中未找到任何 .tar 镜像文件: {images_dir}"
        ));
    }

    let total = files.len();
    let mut loaded = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let file_name = Path::new(file)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| file.clone());
        let _ = window.emit(
            "image-load-progress",
            ImageLoadProgress {
                current: index + 1,
                total,
                image_file: file_name.clone(),
                message: format!("正在导入镜像 [{}/{}] {} ...", index + 1, total, file_name),
                finished: false,
            },
        );

        let load_script = format!("set -e\ndocker load -i {}\n", shell_quote(file));
        let output = run_ssh_script(&host, port, &user, key.as_deref(), &load_script)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("导入镜像 {file_name} 失败: {stderr}"));
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        loaded.push(format!("{file_name}: {stdout}"));
    }

    let _ = window.emit(
        "image-load-progress",
        ImageLoadProgress {
            current: total,
            total,
            image_file: "ALL".to_string(),
            message: "全部离线 Docker 镜像加载完成".to_string(),
            finished: true,
        },
    );
    Ok(loaded)
}

#[tauri::command]
pub async fn remote_docker_compose_action(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    deploy_dir: String,
    action: String,
    service: Option<String>,
) -> Result<ComposeActionResult, String> {
    crate::commands::blocking::run(move || {
        remote_docker_compose_action_blocking(
            host, port, user, key_path, deploy_dir, action, service,
        )
    })
    .await
}

#[tauri::command]
pub async fn check_remote_oceanbase_status(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<OceanBaseHealth, String> {
    crate::commands::blocking::run(move || {
        check_remote_oceanbase_status_blocking(host, port, user, key_path)
    })
    .await
}

#[tauri::command]
pub async fn import_remote_oceanbase_sql(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    sql_file_path: String,
) -> Result<DbImportResult, String> {
    crate::commands::blocking::run(move || {
        import_remote_oceanbase_sql_blocking(host, port, user, key_path, sql_file_path)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_missing_directory_is_not_reported_as_missing_ssh() {
        let message = explain_ssh_failure(
            "bash: line 2: cd: /opt/deploy/deploy: No such file or directory",
            Some("/tmp/key"),
        );
        assert!(message.contains("不存在"));
        assert!(message.contains("原始错误"));
        assert!(!message.contains("OpenSSH"));
    }

    #[test]
    fn missing_ssh_binary_still_mentions_openssh() {
        let message = explain_ssh_failure("bash: line 1: ssh: command not found", None);
        assert!(message.contains("OpenSSH"));
    }

    #[test]
    fn permission_denied_points_at_key_authorization() {
        let message = explain_ssh_failure(
            "root@192.0.2.10: Permission denied (publickey).",
            Some("/tmp/key"),
        );
        assert!(message.contains("认证被拒绝"));
        assert!(!message.contains("OpenSSH"));
    }

    #[test]
    fn install_root_hint_strips_the_declared_compose_directory() {
        assert_eq!(
            install_root_of("/home/tt_net_manager/deploy", "deploy").as_deref(),
            Some("/home/tt_net_manager")
        );
        assert_eq!(
            install_root_of("/srv/demo/deploy/", "deploy").as_deref(),
            Some("/srv/demo")
        );
        assert_eq!(
            install_root_of("/srv/other", "deploy"),
            None,
            "unrelated directories must not produce a misleading suggestion"
        );
    }

    #[test]
    fn only_connection_errors_count_as_ssh_layer_failures() {
        assert!(is_ssh_layer_failure(
            "ssh: connect to host 192.0.2.10 port 22: connection refused"
        ));
        assert!(is_ssh_layer_failure(
            "root@192.0.2.10: permission denied (publickey)."
        ));
        assert!(!is_ssh_layer_failure(
            "bash: line 2: cd: /opt/deploy/deploy: no such file or directory"
        ));
        assert!(!is_ssh_layer_failure(
            "docker: permission denied while trying to connect to the docker daemon socket"
        ));
    }
}
