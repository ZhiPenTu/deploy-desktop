use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use tauri::Emitter;

use super::db_ops::{DbImportResult, OceanBaseHealth};
use super::docker_engine::{DockerInstallResult, DockerInstallStep};
use super::docker_ops::{ComposeActionResult, ImageLoadProgress};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTestResult {
    pub success: bool,
    pub host: String,
    pub user: String,
    pub message: String,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEnvResult {
    pub success: bool,
    pub os: String,
    pub arch: String,
    pub os_release: String,
    pub cpu_cores: usize,
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub disk_available_gb: f64,
    pub docker_installed: bool,
    pub docker_version: Option<String>,
    pub docker_running: bool,
    pub compose_installed: bool,
    pub compose_version: Option<String>,
    pub ports: Vec<RemotePortStatus>,
    pub is_ready: bool,
    pub raw_output: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePortStatus {
    pub port: u16,
    pub protocol: String,
    pub service: String,
    pub available: bool,
    pub occupying_process: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshDeployExecutionResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn test_ssh_connection(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<SshTestResult, String> {
    let target = format!("{user}@{host}:{port}");
    let result =
        crate::commands::blocking::run(move || test_ssh_connection_blocking(host, port, user, key_path))
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

pub(crate) fn test_ssh_connection_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<SshTestResult, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let start_time = std::time::Instant::now();

    let mut cmd = ssh_base_command(&host, port, &user, key.as_deref());
    cmd.arg("echo PONG_DEPLOY_DESKTOP");

    let output = cmd.output();
    let elapsed = start_time.elapsed().as_millis() as u64;

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains("PONG_DEPLOY_DESKTOP") {
                Ok(SshTestResult {
                    success: true,
                    host,
                    user,
                    message: format!("SSH 连接测试成功 (网络往返时延: {}ms)", elapsed),
                    latency_ms: Some(elapsed),
                })
            } else {
                Ok(SshTestResult {
                    success: false,
                    host,
                    user,
                    message: "SSH 连接成功但回显校验不匹配".to_string(),
                    latency_ms: Some(elapsed),
                })
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Ok(SshTestResult {
                success: false,
                host,
                user,
                message: explain_ssh_failure(&stderr, key.as_deref()),
                latency_ms: None,
            })
        }
        Err(e) => Ok(SshTestResult {
            success: false,
            host,
            user,
            message: format!("执行 ssh 命令失败: {}", e),
            latency_ms: None,
        }),
    }
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

pub(crate) fn setup_remote_ssh_key_blocking(
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    key_path: Option<String>,
) -> Result<SshTestResult, String> {
    let target_key = resolve_key_target(key_path, &host);

    let pub_key_path = format!("{}.pub", target_key);

    // 如果私钥不存在，自动生成 ED25519 密钥对
    if !Path::new(&target_key).is_file() {
        if let Some(parent) = Path::new(&target_key).parent() {
            let _ = fs::create_dir_all(parent);
        }
        let gen_res = Command::new(find_binary("ssh-keygen"))
            .args(["-t", "ed25519", "-N", "", "-f", &target_key, "-C", &format!("deploy-desktop@{}", host)])
            .output();
        match gen_res {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                return Err(format!(
                    "生成 SSH 密钥对失败: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Err(e) => return Err(format!("生成 SSH 密钥对失败: {}", e)),
        }
    }

    // 私钥已经可用时直接返回成功，避免重复下发
    if let Ok(result) = test_ssh_connection_blocking(host.clone(), port, user.clone(), Some(target_key.clone())) {
        if result.success {
            return Ok(SshTestResult {
                success: true,
                host,
                user,
                message: format!("当前密钥已可免密登录，无需重复部署。本地私钥: {}", target_key),
                latency_ms: result.latency_ms,
            });
        }
    }

    let pub_content = fs::read_to_string(&pub_key_path)
        .map_err(|e| format!("读取公钥文件 {} 失败: {}", pub_key_path, e))?
        .trim()
        .to_string();

    let remote_sh = remote_authorized_keys_script(&pub_content);
    let mut deploy_error = None;

    if let Some(pwd) = password.filter(|p| !p.is_empty()) {
        match deploy_public_key_with_password(&host, port, &user, &pwd, &remote_sh) {
            Ok(()) => {
                let verify = test_ssh_connection_blocking(host.clone(), port, user.clone(), Some(target_key.clone()))?;
                if verify.success {
                    return Ok(SshTestResult {
                        success: true,
                        host,
                        user,
                        message: format!("公钥已部署并通过免密登录验证。本地私钥: {}", target_key),
                        latency_ms: verify.latency_ms,
                    });
                }
                deploy_error = Some(verify.message);
            }
            Err(e) => deploy_error = Some(e),
        }
    }

    Ok(SshTestResult {
        success: false,
        host,
        user,
        message: match deploy_error {
            Some(e) => format!(
                "自动下发公钥失败: {}\n请手动将以下公钥追加到远程的 ~/.ssh/authorized_keys:\n{}",
                e, pub_content
            ),
            None => format!(
                "未填写初始密码，无法自动下发公钥。请手动将以下公钥追加到远程的 ~/.ssh/authorized_keys:\n{}",
                pub_content
            ),
        },
        latency_ms: None,
    })
}

#[tauri::command]
pub async fn check_remote_env(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<RemoteEnvResult, String> {
    let target = format!("{user}@{host}:{port}");
    let result =
        crate::commands::blocking::run(move || check_remote_env_blocking(host, port, user, key_path))
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

pub(crate) fn check_remote_env_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<RemoteEnvResult, String> {
    let key = resolve_existing_key(key_path, &host)?;

    let script = r#"
echo "OS=$(uname -s 2>/dev/null || echo Linux)"
echo "ARCH=$(uname -m 2>/dev/null || echo x86_64)"
OS_RELEASE_VALUE=$(sed -n 's/^PRETTY_NAME=//p' /etc/os-release 2>/dev/null | tr -d '"' | head -1)
[ -n "$OS_RELEASE_VALUE" ] || OS_RELEASE_VALUE=$(uname -s)
echo "OS_RELEASE=$OS_RELEASE_VALUE"
echo "CPU_CORES=$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null || echo 0)"
echo "MEM_TOTAL_KB=$(awk '/^MemTotal:/{print $2}' /proc/meminfo 2>/dev/null || echo 0)"
echo "MEM_AVAILABLE_KB=$(awk '/^MemAvailable:/{print $2}' /proc/meminfo 2>/dev/null || awk '/^MemFree:/{print $2}' /proc/meminfo 2>/dev/null || echo 0)"
echo "DISK_AVAILABLE_KB=$(df -k /home 2>/dev/null | awk 'NR==2{print $4}' || df -k / 2>/dev/null | awk 'NR==2{print $4}')"
echo "DOCKER_VERSION=$(docker --version 2>/dev/null || true)"
echo "COMPOSE_VERSION=$(docker compose version 2>/dev/null || docker-compose --version 2>/dev/null || true)"
echo "DOCKER_RUNNING=$(docker info >/dev/null 2>&1 && echo yes || echo no)"
for p in 8000 8001 18080 12881 6390 9092 20000; do
  if command -v ss >/dev/null 2>&1; then
    if ss -H -ltnu 2>/dev/null | awk '{print $5}' | grep -q ":$p$"; then
      echo "PORT_${p}=busy"
    else
      echo "PORT_${p}=free"
    fi
  else
    echo "PORT_${p}=unknown"
  fi
done
"#;

    let output = run_ssh_script(&host, port, &user, key.as_deref(), script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key.as_deref()));
    }
    let raw = String::from_utf8_lossy(&output.stdout).to_string();

    let mut os = "Linux".to_string();
    let mut arch = "x86_64".to_string();
    let mut os_release = "Linux".to_string();
    let mut cpu_cores = 0usize;
    let mut total_memory_mb = 0u64;
    let mut available_memory_mb = 0u64;
    let mut disk_available_gb = 0.0f64;
    let mut docker_version = None;
    let mut compose_version = None;
    let mut docker_running = false;
    let mut port_states = std::collections::HashMap::new();
    let mut issues = Vec::new();

    for line in raw.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        match key {
            "OS" => os = value.to_string(),
            "ARCH" => arch = value.to_string(),
            "OS_RELEASE" => os_release = value.to_string(),
            "CPU_CORES" => cpu_cores = value.parse().unwrap_or(0),
            "MEM_TOTAL_KB" => total_memory_mb = value.parse::<u64>().unwrap_or(0) / 1024,
            "MEM_AVAILABLE_KB" => available_memory_mb = value.parse::<u64>().unwrap_or(0) / 1024,
            "DISK_AVAILABLE_KB" => {
                disk_available_gb = value
                    .parse::<f64>()
                    .map(|kb| (kb / 1024.0 / 1024.0 * 10.0).round() / 10.0)
                    .unwrap_or(0.0);
            }
            "DOCKER_VERSION" => {
                if !value.trim().is_empty() {
                    docker_version = Some(value.trim().to_string());
                }
            }
            "COMPOSE_VERSION" => {
                if !value.trim().is_empty() {
                    compose_version = Some(value.trim().to_string());
                }
            }
            "DOCKER_RUNNING" => docker_running = value.trim() == "yes",
            _ => {
                if let Some(port) = key.strip_prefix("PORT_") {
                    port_states.insert(port.to_string(), value.trim().to_string());
                }
            }
        }
    }

    let docker_installed = docker_version.is_some();
    let compose_installed = compose_version.is_some();

    if !docker_installed {
        issues.push("远程机器未安装 Docker".to_string());
    } else if !docker_running {
        issues.push("远程机器 Docker 服务未运行".to_string());
    }

    if !compose_installed {
        issues.push("远程机器缺少 Docker Compose 插件".to_string());
    }

    if total_memory_mb > 0 && total_memory_mb < 8192 {
        issues.push(format!(
            "远程机器物理内存为 {}MB，建议至少 16GB 以平稳运行 OceanBase 容器",
            total_memory_mb
        ));
    }

    if disk_available_gb > 0.0 && disk_available_gb < 20.0 {
        issues.push(format!(
            "远程部署分区磁盘剩余空间仅 {:.1}GB，建议预留 30GB 以上",
            disk_available_gb
        ));
    }

    let core_ports = super::profile::deployment_profile().port_definitions();
    let mut ports = Vec::new();
    for (port, protocol, service) in core_ports {
        let state = port_states
            .get(&port.to_string())
            .map(String::as_str)
            .unwrap_or("unknown");
        let available = state != "busy";
        if state == "busy" {
            issues.push(format!("{} 端口 {} 已被占用", protocol, port));
        }
        ports.push(RemotePortStatus {
            port,
            protocol: protocol.to_string(),
            service: service.to_string(),
            available,
            occupying_process: if state == "busy" {
                Some("远程主机端口监听中".to_string())
            } else {
                None
            },
        });
    }

    let is_ready = docker_installed && docker_running && compose_installed;

    Ok(RemoteEnvResult {
        success: true,
        os,
        arch,
        os_release,
        cpu_cores,
        total_memory_mb,
        available_memory_mb,
        disk_available_gb,
        docker_installed,
        docker_version,
        docker_running,
        compose_installed,
        compose_version,
        ports,
        is_ready,
        raw_output: raw,
        issues,
    })
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

pub(crate) fn run_remote_deploy_action_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    mode: String, // "deploy-all", "server-only", "gateway-app-only", "frontend-only", "status", "start", "stop", "restart"
    target_service: Option<String>,
    deploy_dir: Option<String>,
) -> Result<SshDeployExecutionResult, String> {
    let dir = deploy_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| {
            super::profile::deployment_profile().deploy_dir
        });

    match mode.as_str() {
        "status" | "start" | "stop" | "restart" => {
            let key = resolve_existing_key(key_path, &host)?;
            let action = match mode.as_str() {
                "status" => "ps",
                "start" => "up",
                "stop" => "stop",
                _ => "restart",
            };
            let result = remote_compose_action(&host, port, &user, key.as_deref(), &dir, action, target_service)?;
            Ok(SshDeployExecutionResult {
                success: result.success,
                output: result.output,
                error: result.error,
            })
        }
        "deploy-all" | "server-only" | "gateway-app-only" | "frontend-only" => {
            let Some(script) = find_deploy_script() else {
                return Ok(SshDeployExecutionResult {
                    success: false,
                    output: String::new(),
                    error: Some(
                        "未找到 deploy-lan.sh。构建/同步类操作需要在包含源码和构建环境的仓库根目录执行；打包后的应用仅支持远程 status/start/stop/restart。"
                            .to_string(),
                    ),
                });
            };

            let mut cmd = Command::new("bash");
            cmd.arg(&script);
            let lan_root = dir.strip_suffix("/deploy").unwrap_or(&dir);
            cmd.args([
                "--host",
                &host,
                "--user",
                &user,
                "--port",
                &port.to_string(),
                "--dir",
                lan_root,
            ]);
            if let Some(raw) = key_path.filter(|value| !value.trim().is_empty()) {
                cmd.args(["--key", &expand_tilde(&raw)]);
            }
            cmd.arg(match mode.as_str() {
                "deploy-all" => "--all",
                "server-only" => "--server-only",
                "gateway-app-only" => "--gateway-app-only",
                "frontend-only" => "--frontend-only",
                _ => "--all",
            });

            let output = cmd
                .output()
                .map_err(|e| format!("调度 deploy-lan.sh 失败: {}", e))?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = format!("{}\n{}", stdout, stderr).trim().to_string();

            Ok(SshDeployExecutionResult {
                success: output.status.success(),
                output: combined,
                error: if output.status.success() {
                    None
                } else {
                    Some("部署指令返回非零退出码".to_string())
                },
            })
        }
        other => Ok(SshDeployExecutionResult {
            success: false,
            output: String::new(),
            error: Some(format!("不支持的远程操作: {}", other)),
        }),
    }
}

pub(crate) fn find_binary(name: &str) -> String {
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

fn expand_tilde(raw: &str) -> String {
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

fn default_key_path(host: &str) -> String {
    let home = dirs::home_dir().unwrap_or_else(|| Path::new("/tmp").to_path_buf());
    home.join(".ssh")
        .join(format!("id_ed25519_{}", host.replace('.', "_")))
        .to_string_lossy()
        .to_string()
}

fn resolve_key_target(custom: Option<String>, host: &str) -> String {
    match custom {
        Some(raw) if !raw.trim().is_empty() => expand_tilde(&raw),
        _ => default_key_path(host),
    }
}

pub(crate) fn resolve_existing_key(custom: Option<String>, host: &str) -> Result<Option<String>, String> {
    if let Some(raw) = custom {
        if !raw.trim().is_empty() {
            let path = expand_tilde(&raw);
            if !Path::new(&path).is_file() {
                return Err(format!("指定的私钥文件不存在: {path}"));
            }
            ensure_key_permissions(&path)?;
            return Ok(Some(path));
        }
    }

    let host_specific = default_key_path(host);
    if Path::new(&host_specific).is_file() {
        ensure_key_permissions(&host_specific)?;
        return Ok(Some(host_specific));
    }

    if let Some(home) = dirs::home_dir() {
        for name in ["id_ed25519", "id_rsa"] {
            let candidate = home.join(".ssh").join(name);
            if candidate.is_file() {
                let path = candidate.to_string_lossy().to_string();
                ensure_key_permissions(&path)?;
                return Ok(Some(path));
            }
        }
    }

    Ok(None)
}

#[cfg(unix)]
fn ensure_key_permissions(path: &str) -> Result<(), String> {
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
fn ensure_key_permissions(_path: &str) -> Result<(), String> {
    Ok(())
}

pub(crate) fn ssh_base_command(host: &str, port: u16, user: &str, key: Option<&str>) -> Command {
    let mut cmd = Command::new(find_binary("ssh"));
    cmd.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=8",
        "-o",
        "StrictHostKeyChecking=no",
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

pub(crate) fn run_ssh_script(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    script: &str,
) -> Result<Output, String> {
    let mut cmd = ssh_base_command(host, port, user, key);
    cmd.arg("bash -s");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 ssh 命令失败: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .map_err(|e| format!("写入远程脚本失败: {e}"))?;
    }
    child
        .wait_with_output()
        .map_err(|e| format!("等待远程命令执行失败: {e}"))
}

pub(crate) fn explain_ssh_failure(stderr: &str, key: Option<&str>) -> String {
    let raw = stderr.trim();
    let lower = raw.to_lowercase();

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
    } else if lower.contains("no such file") || lower.contains("command not found") {
        "系统未找到 ssh 命令，请确认 OpenSSH 客户端已安装。"
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

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn remote_authorized_keys_script(public_key: &str) -> String {
    format!(
        r#"set -e
umask 077
mkdir -p ~/.ssh
chmod 700 ~/.ssh
touch ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
KEY={}
grep -qxF "$KEY" ~/.ssh/authorized_keys || printf '%s\n' "$KEY" >> ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
echo KEY_INSTALLED
"#,
        shell_quote(public_key)
    )
}

fn base64_encode(input: &str) -> Result<String, String> {
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

fn deploy_public_key_with_password(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    remote_script: &str,
) -> Result<(), String> {
    let encoded = base64_encode(remote_script)?;
    let remote_command = format!("printf %s {encoded} | base64 -d | bash");
    let sshpass = find_binary("sshpass");

    if Path::new(&sshpass).is_file() {
        let output = Command::new(sshpass)
            .env("SSHPASS", password)
            .args([
                "-e",
                "ssh",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "LogLevel=ERROR",
                "-o",
                "PreferredAuthentications=password,keyboard-interactive",
                "-o",
                "PubkeyAuthentication=no",
                "-o",
                "ConnectTimeout=8",
                "-p",
                &port.to_string(),
                &format!("{user}@{host}"),
                &remote_command,
            ])
            .output()
            .map_err(|e| format!("执行 sshpass 失败: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !stderr.is_empty() {
            return Err(stderr);
        }
    }

    let expect = find_binary("expect");
    if !Path::new(&expect).is_file() {
        return Err("系统未安装 sshpass 或 expect，无法自动使用密码登录".to_string());
    }

    let expect_script = r#"set timeout 25
set password $env(DEPLOY_SSH_PASSWORD)
set host $env(DEPLOY_SSH_HOST)
set port $env(DEPLOY_SSH_PORT)
set user $env(DEPLOY_SSH_USER)
set b64 $env(DEPLOY_SSH_SCRIPT_B64)
spawn ssh -o StrictHostKeyChecking=no -o LogLevel=ERROR -o PreferredAuthentications=password,keyboard-interactive -o PubkeyAuthentication=no -o ConnectTimeout=8 -p $port $user@$host "printf %s $b64 | base64 -d | bash"
expect {
  -re "(?i)password.*:" { send -- "$password\r"; exp_continue }
  -re "(?i)permission denied" { exit 2 }
  -re "(?i)connection refused" { exit 3 }
  timeout { exit 4 }
  eof
}
catch wait result
exit [lindex $result 3]
"#;

    let script_path = std::env::temp_dir().join(format!("deploy-ssh-key-{}.exp", std::process::id()));
    fs::write(&script_path, expect_script)
        .map_err(|e| format!("写入 expect 脚本失败: {e}"))?;
    let output = Command::new(expect)
        .arg(&script_path)
        .env("DEPLOY_SSH_PASSWORD", password)
        .env("DEPLOY_SSH_HOST", host)
        .env("DEPLOY_SSH_PORT", port.to_string())
        .env("DEPLOY_SSH_USER", user)
        .env("DEPLOY_SSH_SCRIPT_B64", &encoded)
        .output();
    let _ = fs::remove_file(&script_path);

    let output = output.map_err(|e| format!("执行 expect 失败: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

fn find_deploy_script() -> Option<String> {
    let mut candidates = Vec::new();
    if let Ok(mut dir) = std::env::current_dir() {
        for _ in 0..4 {
            candidates.push(dir.join("deploy-lan.sh"));
            match dir.parent() {
                Some(parent) => dir = parent.to_path_buf(),
                None => break,
            }
        }
    }
    if let Ok(root) = std::env::var("DEPLOY_DESKTOP_ROOT") {
        candidates.push(Path::new(&root).join("deploy-lan.sh"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("deploy/deploy-lan.sh"));
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().to_string())
}

fn remote_compose_action(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    deploy_dir: &str,
    action: &str,
    service: Option<String>,
) -> Result<ComposeActionResult, String> {
    let service_arg = service
        .filter(|value| !value.trim().is_empty() && value != "all")
        .map(|value| format!(" {}", shell_quote(&value)))
        .unwrap_or_default();
    let action_arg = match action {
        "ps" => "ps".to_string(),
        "up" if service_arg.is_empty() => "up -d --remove-orphans".to_string(),
        "up" => format!("up -d{service_arg}"),
        "stop" => format!("stop{service_arg}"),
        "restart" => format!("restart{service_arg}"),
        "down" => "down".to_string(),
        other => return Err(format!("不支持的 Compose 操作: {other}")),
    };

    // 优先使用交付包声明的清单，找不到时再按常见文件名回退。
    let mut candidates: Vec<String> = Vec::new();
    if let Some(declared) = crate::commands::docker_ops::active_compose_relative() {
        candidates.push(declared.clone());
        candidates.push(format!("deploy/{declared}"));
    }
    for name in [
        "docker-compose.lan.yml",
        "docker-compose.yml",
        "deploy/docker-compose.lan.yml",
        "deploy/docker-compose.yml",
    ] {
        candidates.push(name.to_string());
    }
    let mut unique: Vec<String> = Vec::new();
    for name in candidates {
        if !unique.contains(&name) {
            unique.push(name);
        }
    }
    let list = unique
        .iter()
        .map(|name| shell_quote(name))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        r#"set -e
cd {deploy}
COMPOSE_FILE=""
for candidate in {list}; do
  if [ -f "$candidate" ]; then
    COMPOSE_FILE="$candidate"
    break
  fi
done
if [ -z "$COMPOSE_FILE" ]; then
  echo "目标目录及其 deploy/ 子目录未找到可用的 Compose 清单" >&2
  exit 3
fi
ENV_ARGS=""
for f in .env deploy/.env .env.local deploy/.env.local; do
  if [ -f "$f" ]; then
    ENV_ARGS="$ENV_ARGS --env-file $f"
  fi
done
COMPOSE_ARGS="-f $COMPOSE_FILE"
if docker compose version >/dev/null 2>&1; then
  docker compose $ENV_ARGS $COMPOSE_ARGS {action}
else
  docker-compose $ENV_ARGS $COMPOSE_ARGS {action}
fi
"#,
        deploy = shell_quote(deploy_dir),
        list = list,
        action = action_arg
    );

    let output = run_ssh_script(host, port, user, key, &script)?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let combined = format!("{stdout}\n{stderr}").trim().to_string();
    Ok(ComposeActionResult {
        success: output.status.success(),
        output: combined,
        error: if output.status.success() {
            None
        } else {
            Some(explain_ssh_failure(&stderr, key))
        },
    })
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

pub(crate) fn install_remote_docker_engine_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
) -> Result<DockerInstallResult, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let engine_dir = format!("{}/docker-engine", bundle_dir.trim_end_matches('/'));
    let mut steps = Vec::new();

    let step1 = format!(
        "set -e\nENGINE={}\n[ -d \"$ENGINE\" ] || {{ echo \"未找到离线安装目录: $ENGINE\" >&2; exit 3; }}\necho \"找到离线资源目录: $ENGINE\"\nls -1 \"$ENGINE\"\n",
        shell_quote(&engine_dir)
    );
    let step1_out = run_ssh_script(&host, port, &user, key.as_deref(), &step1)?;
    if !step1_out.status.success() {
        let message = String::from_utf8_lossy(&step1_out.stderr).trim().to_string();
        return Ok(DockerInstallResult {
            success: false,
            steps: vec![DockerInstallStep {
                step: "1. 扫描 Docker 离线静态二进制包".to_string(),
                status: "failed".to_string(),
                message,
            }],
            docker_version: None,
            error_message: Some("远程主机缺少 docker-engine 离线安装目录".to_string()),
        });
    }
    steps.push(DockerInstallStep {
        step: "1. 扫描 Docker 离线静态二进制包".to_string(),
        status: "success".to_string(),
        message: format!("找到离线资源目录: {engine_dir}"),
    });

    let step2 = format!(
        r##"set -e
ENGINE={}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
TGZ=$(find "$ENGINE" -maxdepth 1 -type f \( -name '*.tgz' -o -name '*.tar.gz' \) | head -1)
if [ -n "$TGZ" ]; then
  tar -zxf "$TGZ" -C "$TMP"
  SRC="$TMP/docker"
else
  SRC="$ENGINE"
fi
if [ ! -f "$SRC/docker" ] && [ ! -f "$SRC/dockerd" ]; then
  echo "未找到 docker 静态二进制文件" >&2
  exit 4
fi
cp -a "$SRC/." /usr/bin/
for binary in docker dockerd containerd runc ctr; do
  [ -f "/usr/bin/$binary" ] && chmod +x "/usr/bin/$binary"
done
if [ -f "$ENGINE/docker-compose" ]; then
  mkdir -p /usr/local/bin /usr/libexec/docker/cli-plugins
  cp -f "$ENGINE/docker-compose" /usr/local/bin/docker-compose
  chmod +x /usr/local/bin/docker-compose
  cp -f "$ENGINE/docker-compose" /usr/libexec/docker/cli-plugins/docker-compose
  chmod +x /usr/libexec/docker/cli-plugins/docker-compose
fi
echo DOCKER_BINARIES_READY
"##,
        shell_quote(&engine_dir)
    );
    let step2_out = run_ssh_script(&host, port, &user, key.as_deref(), &step2)?;
    if !step2_out.status.success() {
        let message = String::from_utf8_lossy(&step2_out.stderr).trim().to_string();
        steps.push(DockerInstallStep {
            step: "2. 安装 Docker 核心二进制文件".to_string(),
            status: "failed".to_string(),
            message,
        });
        return Ok(DockerInstallResult {
            success: false,
            steps,
            docker_version: None,
            error_message: Some("远程主机安装 Docker 二进制失败".to_string()),
        });
    }
    steps.push(DockerInstallStep {
        step: "2. 安装 Docker 核心二进制文件".to_string(),
        status: "success".to_string(),
        message: "Docker 与 Compose 二进制已部署至远程主机".to_string(),
    });

    let step3 = r##"set -e
mkdir -p /etc/systemd/system /etc/docker
cat > /etc/systemd/system/docker.service <<'UNIT'
[Unit]
Description=Docker Application Container Engine
Documentation=https://docs.docker.com
After=network-online.target firewalld.service containerd.service
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/bin/dockerd
ExecReload=/bin/kill -s HUP $MAINPID
TimeoutSec=0
RestartSec=2
Restart=always
LimitNOFILE=infinity
LimitNPROC=infinity
LimitCORE=infinity
TasksMax=infinity
Delegate=yes
KillMode=process
OOMScoreAdjust=-500

[Install]
WantedBy=multi-user.target
UNIT
if [ ! -f /etc/docker/daemon.json ]; then
  cat > /etc/docker/daemon.json <<'JSON'
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "100m",
    "max-file": "3"
  },
  "live-restore": true
}
JSON
fi
echo SYSTEMD_CONFIG_READY
"##;
    let step3_out = run_ssh_script(&host, port, &user, key.as_deref(), step3)?;
    if !step3_out.status.success() {
        let message = String::from_utf8_lossy(&step3_out.stderr).trim().to_string();
        steps.push(DockerInstallStep {
            step: "3. 配置 systemd 服务与 Daemon 参数".to_string(),
            status: "failed".to_string(),
            message,
        });
        return Ok(DockerInstallResult {
            success: false,
            steps,
            docker_version: None,
            error_message: Some("远程主机写入 Docker systemd 配置失败".to_string()),
        });
    }
    steps.push(DockerInstallStep {
        step: "3. 配置 systemd 服务与 Daemon 参数".to_string(),
        status: "success".to_string(),
        message: "systemd 单元与 daemon.json 已写入".to_string(),
    });

    let step4 = "set -e\nsystemctl daemon-reload\nsystemctl enable --now docker\ndocker --version\n";
    let step4_out = run_ssh_script(&host, port, &user, key.as_deref(), step4)?;
    if !step4_out.status.success() {
        let message = String::from_utf8_lossy(&step4_out.stderr).trim().to_string();
        steps.push(DockerInstallStep {
            step: "4. 启动 Docker 服务并验证运行状态".to_string(),
            status: "failed".to_string(),
            message,
        });
        return Ok(DockerInstallResult {
            success: false,
            steps,
            docker_version: None,
            error_message: Some("远程主机启动 Docker 服务失败".to_string()),
        });
    }
    let docker_version = String::from_utf8_lossy(&step4_out.stdout).trim().to_string();
    steps.push(DockerInstallStep {
        step: "4. 启动 Docker 服务并验证运行状态".to_string(),
        status: "success".to_string(),
        message: format!("Docker 服务已启动: {docker_version}"),
    });

    Ok(DockerInstallResult {
        success: true,
        steps,
        docker_version: Some(docker_version),
        error_message: None,
    })
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
        return Err(format!("远程镜像目录中未找到任何 .tar 镜像文件: {images_dir}"));
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

pub(crate) fn remote_docker_compose_action_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    deploy_dir: String,
    action: String,
    service: Option<String>,
) -> Result<ComposeActionResult, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let mut result = remote_compose_action(
        &host,
        port,
        &user,
        key.as_deref(),
        &deploy_dir,
        &action,
        service,
    )?;
    if action == "up" && result.success {
        let containers = super::docker_ops::artifact_containers();
        if !containers.is_empty() {
            let names = containers
                .iter()
                .map(|name| shell_quote(name))
                .collect::<Vec<_>>()
                .join(" ");
            let restart = run_ssh_script(
                &host,
                port,
                &user,
                key.as_deref(),
                &format!("docker restart {names}"),
            )?;
            if restart.status.success() {
                result.output.push_str(&format!(
                    "\n已重启交付产物容器: {}",
                    containers.join(", ")
                ));
            } else {
                let stderr = String::from_utf8_lossy(&restart.stderr).trim().to_string();
                result.success = false;
                result.error = Some(format!("重启交付产物容器失败: {stderr}"));
            }
        }
    }
    Ok(result)
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

pub(crate) fn check_remote_oceanbase_status_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<OceanBaseHealth, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let (profile, database) = super::profile::require_database()?;
    let container = shell_quote(&profile.container_name(&database.service_key));
    let client = shell_quote(&database.client);
    let db_port = database.port;
    let script = format!(
        r#"set -e
STATE=$(docker inspect --format '{{{{.State.Status}}}}|{{{{if .State.Health}}}}{{{{.State.Health.Status}}}}{{{{else}}}}none{{{{end}}}}' {container} 2>/dev/null || echo "not_found|not_found")
echo "STATE=$STATE"
RUNNING=$(echo "$STATE" | cut -d'|' -f1)
if [ "$RUNNING" = "running" ]; then
  if docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot -e 'SELECT 1;' >/dev/null 2>&1; then
    echo "READY=yes"
  else
    echo "READY=no"
  fi
else
  echo "READY=no"
fi
"#,
        container = container,
        client = client,
        db_port = db_port
    );
    let output = run_ssh_script(&host, port, &user, key.as_deref(), &script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key.as_deref()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut state = "not_found".to_string();
    let mut health = "not_found".to_string();
    let mut ready = false;
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("STATE=") {
            let mut parts = value.split('|');
            state = parts.next().unwrap_or("not_found").to_string();
            health = parts.next().unwrap_or("none").to_string();
        } else if let Some(value) = line.strip_prefix("READY=") {
            ready = value.trim() == "yes";
        }
    }

    let is_running = state == "running";
    Ok(OceanBaseHealth {
        is_running,
        is_ready: ready,
        container_health: health,
        message: if !is_running {
            "OceanBase 容器未运行或未启动".to_string()
        } else if ready {
            "OceanBase 数据库连接正常，已就绪可执行迁移".to_string()
        } else {
            "OceanBase 正在启动初始化中，请等待其变为 healthy 状态".to_string()
        },
    })
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

pub(crate) fn import_remote_oceanbase_sql_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    sql_file_path: String,
) -> Result<DbImportResult, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let (profile, database) = super::profile::require_database()?;
    let container = shell_quote(&profile.container_name(&database.service_key));
    let client = shell_quote(&database.client);
    let db_port = database.port;
    let database = database.name.clone();
    let script = format!(
        r#"set -e
SQL={}
if [ ! -f "$SQL" ]; then
  echo "远程 SQL 脚本不存在: $SQL" >&2
  exit 4
fi
docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot -e "CREATE DATABASE IF NOT EXISTS {database} DEFAULT CHARACTER SET utf8mb4;"
docker exec -i {container} {client} -h127.0.0.1 -P{db_port} -uroot --default-character-set=utf8mb4 {database} < "$SQL"
echo "TABLE_COUNT=$(docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot -N -e "SELECT count(*) FROM information_schema.tables WHERE table_schema='{database}';")"
echo "DICT_COUNT=$(docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot {database} -N -e 'SELECT count(*) FROM parameter_dict;' 2>/dev/null || echo 0)"
echo "USER_COUNT=$(docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot {database} -N -e 'SELECT count(*) FROM users;' 2>/dev/null || echo 0)"
docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot -N -e "SELECT table_name FROM information_schema.tables WHERE table_schema='{database}' ORDER BY table_name;" | while read -r table_name; do
  [ -n "$table_name" ] && echo "TABLE_NAME=$table_name"
done
"#,
        shell_quote(&sql_file_path),
        container = container,
        client = client,
        db_port = db_port,
        database = database
    );
    let output = run_ssh_script(&host, port, &user, key.as_deref(), &script)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        return Ok(DbImportResult {
            success: false,
            table_count: 0,
            dictionary_count: 0,
            user_count: 0,
            tables: Vec::new(),
            message: "远程 SQL 导入执行出错".to_string(),
            error: Some(if stderr.is_empty() {
                explain_ssh_failure("", key.as_deref())
            } else {
                stderr
            }),
        });
    }

    let mut table_count = 0usize;
    let mut dictionary_count = 0usize;
    let mut user_count = 0usize;
    let mut tables = Vec::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("TABLE_COUNT=") {
            table_count = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("DICT_COUNT=") {
            dictionary_count = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("USER_COUNT=") {
            user_count = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("TABLE_NAME=") {
            let name = value.trim();
            if !name.is_empty() {
                tables.push(name.to_string());
            }
        }
    }

    Ok(DbImportResult {
        success: true,
        table_count,
        dictionary_count,
        user_count,
        tables,
        message: format!(
            "数据库迁移完成！成功同步 {} 张数据表、{} 条字典数据与 {} 个系统用户",
            table_count, dictionary_count, user_count
        ),
        error: None,
    })
}
