//! SSH 连接与远程脚本执行的公共能力。
//!
//! 其他命令模块通过 `commands::remote_ssh` 使用这里的命令拼装、超时与错误解释，
//! 不各自实现一份 SSH 调用。

use std::process::{Command, Output, Stdio};

use super::*;

pub(crate) fn find_binary(name: &str) -> String {
    if name == "docker" {
        return crate::commands::docker_setup::find_docker_binary()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| name.to_string());
    }
    let candidates = [
        format!("/usr/bin/{name}"),
        format!("/bin/{name}"),
        format!("/usr/local/bin/{name}"),
        format!("/opt/homebrew/bin/{name}"),
    ];
    candidates
        .into_iter()
        .find(|candidate| Path::new(candidate).is_file())
        .unwrap_or_else(|| name.to_string())
}

pub(crate) fn expand_tilde(raw: &str) -> String {
    let value = raw.trim();
    if value == "~" {
        return dirs::home_dir()
            .map(|home| home.to_string_lossy().to_string())
            .unwrap_or_else(|| value.to_string());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    value.to_string()
}

pub(crate) fn default_key_path(host: &str) -> String {
    let home = dirs::home_dir().unwrap_or_else(|| Path::new("/tmp").to_path_buf());
    home.join(".ssh")
        .join(format!("id_ed25519_{}", host.replace('.', "_")))
        .to_string_lossy()
        .to_string()
}

pub(crate) fn resolve_key_target(custom: Option<String>, host: &str) -> String {
    match custom {
        Some(raw) if !raw.trim().is_empty() => expand_tilde(&raw),
        _ => default_key_path(host),
    }
}

#[cfg(unix)]
pub(crate) fn ensure_key_permissions(path: &str) -> Result<(), String> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    if let Ok(meta) = fs::metadata(path) {
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(format!(
                "私钥文件权限过宽: {path} (当前 {mode:o})，请先执行 chmod 600 {path}"
            ));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn ensure_key_permissions(_path: &str) -> Result<(), String> {
    Ok(())
}

fn ssh_base_command_with_policy(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    strict_host_key: bool,
) -> Command {
    let mut cmd = Command::new(find_binary("ssh"));
    cmd.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=8",
        "-o",
        if strict_host_key {
            "StrictHostKeyChecking=yes"
        } else {
            "StrictHostKeyChecking=no"
        },
        "-o",
        "LogLevel=ERROR",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=4",
        "-p",
        &port.to_string(),
    ]);
    if let Some(key) = key {
        cmd.args(["-i", key]);
    }
    cmd.arg(format!("{user}@{host}"));
    cmd
}

pub(crate) fn ssh_base_command(host: &str, port: u16, user: &str, key: Option<&str>) -> Command {
    ssh_base_command_with_policy(host, port, user, key, false)
}

pub(crate) fn known_host_token(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

pub(crate) fn ensure_known_host(host: &str, port: u16) -> Result<(), String> {
    let token = known_host_token(host, port);
    let output = Command::new(find_binary("ssh-keygen"))
        .args(["-F", &token])
        .output()
        .map_err(|_| String::from("无法确认目标主机身份，请检查本机 OpenSSH 配置后重试"))?;
    if output.status.success() && !output.stdout.is_empty() {
        return Ok(());
    }
    Err(format!(
        "目标主机 {token} 的 SSH 指纹尚未在本机确认。发送镜像仓库凭据前，请先用系统 SSH 连接并确认主机指纹，或改用离线镜像包。"
    ))
}

pub(crate) fn run_ssh_script(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    script: &str,
) -> Result<Output, String> {
    run_ssh_script_with_timeout(
        host,
        port,
        user,
        key,
        script,
        std::time::Duration::from_secs(600),
    )
}

pub(crate) fn run_ssh_script_with_timeout(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    script: &str,
    timeout: std::time::Duration,
) -> Result<Output, String> {
    run_ssh_script_with_policy(
        host,
        port,
        user,
        key,
        script,
        timeout,
        false,
    )
}

pub(crate) fn run_ssh_script_strict_with_timeout(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    script: &str,
    timeout: std::time::Duration,
) -> Result<Output, String> {
    run_ssh_script_with_policy(
        host,
        port,
        user,
        key,
        script,
        timeout,
        true,
    )
}

fn run_ssh_script_with_policy(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    script: &str,
    timeout: std::time::Duration,
    strict_host_key: bool,
) -> Result<Output, String> {
    use crate::core::{process, shell};
    let mut cmd = ssh_base_command_with_policy(host, port, user, key, strict_host_key);
    cmd.arg(shell::BASH_STDIN);
    process::run(
        cmd,
        process::Input::Bytes(remote_programme(script)),
        process::Options {
            timeout,
            ..process::Options::default()
        },
    )
}

/// 所有远程脚本统一先加载 Docker 访问入口，脚本内部继续按普通命令书写。
fn remote_programme(script: &str) -> Vec<u8> {
    crate::core::shell::stdin_program(&format!(
        "{}\n{}",
        crate::commands::docker_setup::REMOTE_DOCKER_PREAMBLE,
        script
    ))
}

pub(crate) fn explain_ssh_failure(stderr: &str, key: Option<&str>) -> String {
    let raw = stderr.trim();
    let lower = raw.to_lowercase();

    if crate::commands::docker_setup::docker_socket_permission_denied(raw) {
        return String::from(
            "Docker 访问失败：当前 SSH 用户无权访问 Docker。请执行 sudo usermod -aG docker <用户名> 后重新登录，或为该用户配置免密 sudo docker。",
        );
    }

    let hint = if lower.contains("no route to host") {
        "无法路由到目标主机。请确认工作站与目标机在同一局域网，并在 macOS「系统设置 → 隐私与安全性 → 本地网络」中允许本应用访问本地网络。"
    } else if lower.contains("connection refused") {
        "目标主机可达，但 SSH 端口未监听或端口号填写错误。"
    } else if lower.contains("permission denied") {
        if key.is_some() {
            "认证被拒绝：目标机未授权当前密钥。请先配置免密登录，或手动把公钥追加到目标机 ~/.ssh/authorized_keys。"
        } else {
            "认证被拒绝：未找到可用私钥。请先配置免密 SSH 密钥。"
        }
    } else if lower.contains("timed out") {
        "连接超时：请检查网络连通性，以及目标机防火墙是否放行 SSH 端口。"
    } else if lower.contains("host key verification failed") {
        "目标主机指纹与本地 known_hosts 记录不一致。请执行 ssh-keygen -R <目标主机> 后重试。"
    } else if lower.contains("command not found: ssh") || lower.contains("ssh: command not found") {
        "系统未找到 ssh 命令，请确认 OpenSSH 客户端已安装。"
    } else if lower.contains("no such file or directory") {
        "目标机上不存在命令需要的文件或目录。请确认部署目录与实际安装目录一致，或先完成首次部署。"
    } else if lower.contains("command not found") {
        "目标机缺少执行部署所需的命令，请先确认目标机已安装 Docker 与 Compose。"
    } else {
        ""
    };

    if hint.is_empty() {
        if raw.is_empty() {
            "连接失败：SSH 未返回错误信息，请检查目标主机状态与网络。".to_string()
        } else {
            format!("连接失败: {raw}")
        }
    } else if raw.is_empty() {
        format!("连接失败：{hint}")
    } else {
        format!("连接失败：{hint}\n原始错误: {raw}")
    }
}

pub(crate) use crate::core::shell::quote as shell_quote;

pub(crate) fn base64_encode(input: &str) -> Result<String, String> {
    let mut child = Command::new(find_binary("base64"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("调用 base64 失败: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("写入 base64 输入失败: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 base64 结束失败: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_host_token_includes_non_default_port() {
        assert_eq!(known_host_token("host.example.com", 22), "host.example.com");
        assert_eq!(
            known_host_token("host.example.com", 2222),
            "[host.example.com]:2222"
        );
    }

    #[test]
    fn docker_socket_permission_is_not_reported_as_ssh_auth_failure() {
        let message = explain_ssh_failure(
            "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock",
            Some("/tmp/key"),
        );
        assert!(message.contains("无权访问 Docker"));
        assert!(!message.contains("目标机未授权当前密钥"));
    }
}
