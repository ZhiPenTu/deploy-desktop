//! 远程环境检查与部署执行。
//!
//! 这里负责判断目标机上 Docker、Compose 目录与部署脚本的实际情况，并把失败原因
//! 翻译成用户能处理的信息；不猜测部署位置，找不到就明确报告缺少什么。

use std::process::Command;

use super::*;

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

    let core_ports = crate::commands::profile::deployment_profile().port_definitions();
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
        .unwrap_or_else(|| crate::commands::profile::deployment_profile().deploy_dir);

    match mode.as_str() {
        "status" | "start" | "stop" | "restart" => {
            let key = resolve_existing_key(key_path, &host)?;
            let action = match mode.as_str() {
                "status" => "ps",
                "start" => "up",
                "stop" => "stop",
                _ => "restart",
            };
            let result = remote_compose_action(
                &host,
                port,
                &user,
                key.as_deref(),
                &dir,
                action,
                target_service,
            )?;
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

pub(crate) fn find_deploy_script() -> Option<String> {
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

/// 部署目录不存在时返回的标记，便于把这类错误与其他远程命令失败区分开。
pub(crate) const DEPLOY_DIR_MISSING: &str = "DEPLOY_DIR_MISSING";

/// 远程错误是否发生在 SSH 连接层：只有这类错误才用连接类提示解释。
pub(crate) fn is_ssh_layer_failure(lower: &str) -> bool {
    let auth_failure = lower.contains("permission denied")
        && (lower.contains("publickey")
            || lower.contains("password")
            || lower.contains("keyboard-interactive"));
    auth_failure
        || lower.contains("no route to host")
        || lower.contains("connection refused")
        || lower.contains("timed out")
        || lower.contains("host key verification failed")
        || lower.contains("network is unreachable")
        || lower.contains("connection closed by")
        || lower.contains("connection reset by")
}

/// 目标机上实际运行服务所在的 Compose 目录；同一前缀对应多个目录时不作推断。
pub(crate) fn detect_remote_compose_dir(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    container_prefix: &str,
) -> Option<String> {
    let prefix = container_prefix.trim();
    if prefix.is_empty() {
        return None;
    }
    let script = format!(
        "docker ps --filter name={filter} --format '{{{{.Label \"com.docker.compose.project.working_dir\"}}}}' 2>/dev/null | sed '/^$/d' | sort -u | head -n 2\n",
        filter = shell_quote(prefix)
    );
    let output = run_ssh_script(host, port, user, key, &script).ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut dirs = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let first = dirs.next()?.to_string();
    if dirs.next().is_some() {
        return None;
    }
    Some(first)
}

/// 部署目录由安装根目录与包内清单所在目录拼接而成；能反推时返回安装根目录。
pub(crate) fn install_root_of(dir: &str, relative: &str) -> Option<String> {
    let dir = dir.trim_end_matches('/');
    let relative = relative.trim().trim_matches('/');
    if relative.is_empty() {
        return Some(dir.to_string());
    }
    dir.strip_suffix(&format!("/{relative}"))
        .filter(|hint| !hint.is_empty())
        .map(str::to_string)
}

/// 部署目录不含 Compose 清单时，定位实际部署位置并给出应填写的安装根目录。
pub(crate) fn missing_deploy_dir_message(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    deploy_dir: &str,
) -> String {
    let bundle = crate::commands::projects::active_bundle();
    let relative = bundle
        .as_deref()
        .map(|bundle| bundle.deploy_source_rel())
        .unwrap_or_default();
    let compose_name = bundle
        .as_deref()
        .map(|bundle| bundle.effective_compose())
        .and_then(|path| {
            Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "docker-compose.yml".to_string());
    let prefix = crate::commands::profile::deployment_profile().container_prefix;
    // 运行中的容器记录真实工作目录；容器未运行时先看所选目录本身，
    // 再看上级安装根目录是否已有清单，避免把「清单所在目录」当作安装根目录。
    let detected = detect_remote_compose_dir(host, port, user, key, &prefix)
        .filter(|dir| !same_dir(dir, deploy_dir))
        .or_else(|| {
            if detect_compose_in_directory(host, port, user, key, deploy_dir, &compose_name)
                .is_some()
            {
                return Some(deploy_dir.trim_end_matches('/').to_string());
            }
            let install = install_root_of(deploy_dir, &relative)?;
            detect_compose_in_directory(host, port, user, key, &install, &compose_name)
        });
    let Some(detected) = detected else {
        return format!(
            "目标机未在 {deploy_dir} 找到 Compose 清单，请在「项目管理」中确认部署目录填写的是安装根目录。"
        );
    };
    match install_root_of(&detected, &relative) {
        Some(install) => format!(
            "目标机未在 {deploy_dir} 找到 Compose 清单。检测到 Compose 清单位于 {detected}，对应的安装根目录是 {install}，请在「项目管理」中确认部署目录填写该安装根目录后重试。"
        ),
        None => format!(
            "目标机未在 {deploy_dir} 找到 Compose 清单。检测到 Compose 清单位于 {detected}，请在「项目管理」中确认部署目录填写的是安装根目录后重试。"
        ),
    }
}

pub(crate) fn same_dir(left: &str, right: &str) -> bool {
    left.replace('\\', "/").trim_end_matches('/') == right.replace('\\', "/").trim_end_matches('/')
}

pub(crate) fn detect_compose_in_directory(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    directory: &str,
    compose_name: &str,
) -> Option<String> {
    let script = format!(
        "DIR={dir}\nfor name in {first} 'docker-compose.yml'; do\n  if [ -f \"$DIR/$name\" ]; then printf 'DETECTED=%s\\n' \"$DIR\"; exit 0; fi\ndone\n",
        dir = shell_quote(directory),
        first = shell_quote(compose_name)
    );
    let output = run_ssh_script(host, port, user, key, &script).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("DETECTED=").map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn compose_action_error(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    deploy_dir: &str,
    stderr: &str,
    stdout: &str,
) -> String {
    let raw = stderr.trim();
    if raw.contains(DEPLOY_DIR_MISSING) {
        return missing_deploy_dir_message(host, port, user, key, deploy_dir);
    }
    if is_ssh_layer_failure(&raw.to_lowercase()) {
        return explain_ssh_failure(raw, key);
    }
    let detail = if raw.is_empty() { stdout.trim() } else { raw };
    if detail.is_empty() {
        "Compose 操作未返回错误信息，请检查目标机的容器状态后重试。".to_string()
    } else {
        format!("Compose 操作未成功：{detail}")
    }
}

pub(crate) fn remote_compose_action(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    deploy_dir: &str,
    action: &str,
    service: Option<String>,
) -> Result<ComposeActionResult, String> {
    let target_service = service
        .as_deref()
        .filter(|value| !value.trim().is_empty() && *value != "all")
        .unwrap_or("全部")
        .to_string();
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
    for name in crate::core::compose::SEARCH_CANDIDATES {
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
    log::info!(
        target: "compose",
        "远程 Compose 操作：动作={action} 目标={user}@{host}:{port} 目录={deploy_dir} 服务={target_service}"
    );
    let script = format!(
        r#"set -e
DEPLOY={deploy}
if [ ! -d "$DEPLOY" ]; then
  echo "{missing}" >&2
  exit 21
fi
cd "$DEPLOY"
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
        missing = DEPLOY_DIR_MISSING,
        list = list,
        action = action_arg
    );

    let output = run_ssh_script(host, port, user, key, &script)?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let combined = format!("{stdout}\n{stderr}").trim().to_string();
    let success = output.status.success();
    let error = if success {
        None
    } else {
        Some(compose_action_error(
            host, port, user, key, deploy_dir, &stderr, &stdout,
        ))
    };
    match &error {
        Some(message) => log::error!(
            target: "compose",
            "远程 Compose 操作失败：动作={action} 目录={deploy_dir} {message}"
        ),
        None => log::info!(
            target: "compose",
            "远程 Compose 操作完成：动作={action} 目录={deploy_dir}"
        ),
    }
    Ok(ComposeActionResult {
        success,
        output: combined,
        error,
    })
}
