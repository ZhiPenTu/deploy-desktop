use super::bundle::normalize_install_dir;
use super::models::{RemoteArtifactInfo, RemoteDeploymentStatus};
use crate::commands::docker_ops::ContainerStatusItem;
use crate::commands::remote_ssh::{
    explain_ssh_failure, resolve_existing_key, run_ssh_script, shell_quote,
};
use std::collections::HashMap;

#[tauri::command]
pub async fn inspect_remote_deployment(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    install_dir: String,
) -> Result<RemoteDeploymentStatus, String> {
    crate::commands::blocking::run(move || {
        inspect_remote_deployment_blocking(host, port, user, key_path, install_dir)
    })
    .await
}

pub(crate) fn inspect_remote_deployment_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    install_dir: String,
) -> Result<RemoteDeploymentStatus, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let install_dir = normalize_install_dir(&install_dir);
    inspect_remote_deployment_inner(&host, port, &user, key.as_deref(), &install_dir)
}

#[tauri::command]
pub async fn get_remote_containers_status(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    install_dir: String,
) -> Result<Vec<ContainerStatusItem>, String> {
    crate::commands::blocking::run(move || {
        get_remote_containers_status_blocking(host, port, user, key_path, install_dir)
    })
    .await
}

pub(crate) fn get_remote_containers_status_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    install_dir: String,
) -> Result<Vec<ContainerStatusItem>, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let install_dir = normalize_install_dir(&install_dir);
    let status = inspect_remote_deployment_inner(&host, port, &user, key.as_deref(), &install_dir)?;
    Ok(status.containers)
}

#[tauri::command]
pub async fn get_remote_service_logs(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    container_name: String,
    tail: u32,
) -> Result<String, String> {
    crate::commands::blocking::run(move || {
        get_remote_service_logs_blocking(host, port, user, key_path, container_name, tail)
    })
    .await
}

pub(crate) fn get_remote_service_logs_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    container_name: String,
    tail: u32,
) -> Result<String, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let tail = tail.clamp(1, 5000);
    let script = format!(
        "docker logs --tail {} {} 2>&1",
        tail,
        shell_quote(container_name.trim())
    );
    let output = run_ssh_script(&host, port, &user, key.as_deref(), &script)?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let combined = if stdout.is_empty() { stderr } else { stdout };
    if output.status.success() {
        Ok(combined)
    } else if combined.is_empty() {
        Err("读取远程日志失败，目标容器可能不存在".to_string())
    } else {
        Err(combined)
    }
}
pub(super) fn ensure_required_containers(
    components: &[String],
    status: &RemoteDeploymentStatus,
) -> Result<(), String> {
    let profile = crate::commands::profile::deployment_profile();
    let mut required: Vec<String> = Vec::new();
    for component in components {
        required.extend(profile.component_services(component));
    }

    let missing: Vec<String> = required
        .into_iter()
        .filter(|key| {
            !status
                .containers
                .iter()
                .any(|container| container.service_key == *key && container.state != "not_created")
        })
        .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "目标机缺少以下容器：{}。请先使用部署向导完成首次安装，或确认部署目录是否正确。",
            missing.join("、")
        ))
    }
}

pub(super) fn inspect_remote_deployment_inner(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install_dir: &str,
) -> Result<RemoteDeploymentStatus, String> {
    let deploy_dir = format!("{install_dir}/deploy");
    let mut script = format!(
        r#"set -e
INSTALL={install}
DEPLOY={deploy}
echo "INSTALL_DIR=$INSTALL"
echo "DEPLOY_DIR=$DEPLOY"
if [ -d "$DEPLOY" ]; then echo "DEPLOY_EXISTS=yes"; else echo "DEPLOY_EXISTS=no"; fi
if [ -f "$DEPLOY/docker-compose.lan.yml" ]; then
  echo "COMPOSE_FILE=docker-compose.lan.yml"
elif [ -f "$DEPLOY/docker-compose.yml" ]; then
  echo "COMPOSE_FILE=docker-compose.yml"
else
  echo "COMPOSE_FILE="
fi
if command -v docker >/dev/null 2>&1; then echo "DOCKER=yes"; else echo "DOCKER=no"; fi
if docker info >/dev/null 2>&1; then echo "DOCKER_RUNNING=yes"; else echo "DOCKER_RUNNING=no"; fi
if docker compose version >/dev/null 2>&1; then echo "COMPOSE=yes"; elif command -v docker-compose >/dev/null 2>&1; then echo "COMPOSE=yes"; else echo "COMPOSE=no"; fi
artifact() {{
  local key=$1
  local path=$2
  if [ -e "$path" ]; then
    echo "ART_${{key}}_EXISTS=yes"
    if [ -f "$path" ]; then
      echo "ART_${{key}}_SIZE=$(stat -c '%s' "$path" 2>/dev/null || echo 0)"
    else
      echo "ART_${{key}}_SIZE=0"
    fi
    echo "ART_${{key}}_MTIME=$(stat -c '%y' "$path" 2>/dev/null | cut -d. -f1)"
  else
    echo "ART_${{key}}_EXISTS=no"
    echo "ART_${{key}}_SIZE=0"
    echo "ART_${{key}}_MTIME="
  fi
}}
artifact SERVER_JAR "$INSTALL/backend-java/server/target/{artifact}"
artifact GATEWAY_CLASSES "$INSTALL/backend-java/gateway-app/target/classes"
artifact GATEWAY_LIB "$INSTALL/backend-java/gateway-app/target/lib"
artifact FRONTEND_DIST "$DEPLOY/frontend_dist"
echo "---CONTAINERS---"
"#,
        install = shell_quote(install_dir),
        deploy = shell_quote(&deploy_dir),
        artifact = crate::commands::profile::deployment_profile().backend_artifact
    );

    for (container, _) in crate::commands::profile::deployment_profile().container_entries() {
        script.push_str(&format!(
            r#"c={container}
state=$(docker inspect --format '{{{{.State.Status}}}}' "$c" 2>/dev/null || echo missing)
health=$(docker inspect --format '{{{{if .State.Health}}}}{{{{.State.Health.Status}}}}{{{{else}}}}none{{{{end}}}}' "$c" 2>/dev/null || echo none)
image=$(docker inspect --format '{{{{.Config.Image}}}}' "$c" 2>/dev/null || echo "")
line=$(docker ps -a --filter "name=^${{c}}$" --format '{{{{.Status}}}}|{{{{.Ports}}}}' 2>/dev/null | head -1)
status_text=${{line%%|*}}
ports=${{line#*|}}
if [ "$ports" = "$line" ]; then ports=""; fi
printf 'CONTAINER|%s|%s|%s|%s|%s|%s\n' "$c" "$state" "$health" "$image" "$status_text" "$ports"
"#,
            container = shell_quote(&container)
        ));
    }

    let output = run_ssh_script(host, port, user, key, &script)
        .map_err(|e| format!("连接目标机失败: {e}"))?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key));
    }

    let mut flags: HashMap<String, String> = HashMap::new();
    let mut containers: Vec<ContainerStatusItem> = Vec::new();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("CONTAINER|") {
            let parts: Vec<&str> = rest.splitn(6, '|').collect();
            if parts.len() == 6 {
                let name = parts[0].trim().to_string();
                let state = parts[1].trim().to_string();
                let health = parts[2].trim().to_string();
                let image = parts[3].trim().to_string();
                let status_text = parts[4].trim().to_string();
                let ports = parts[5].trim().to_string();
                let service_key =
                    crate::commands::profile::deployment_profile().service_key_for_container(&name);
                let (state, health, status_text) = if state == "missing" {
                    (
                        "not_created".to_string(),
                        "none".to_string(),
                        "容器未创建".to_string(),
                    )
                } else {
                    (state, health, status_text)
                };
                containers.push(ContainerStatusItem {
                    name,
                    service_key,
                    state,
                    health,
                    status_text,
                    ports,
                    image,
                });
            }
        } else if let Some((key, value)) = line.split_once('=') {
            flags.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    let deploy_exists = flags
        .get("DEPLOY_EXISTS")
        .map(|v| v == "yes")
        .unwrap_or(false);
    let compose_file = flags.get("COMPOSE_FILE").cloned().unwrap_or_default();
    let docker_available = flags.get("DOCKER").map(|v| v == "yes").unwrap_or(false);
    let docker_running = flags
        .get("DOCKER_RUNNING")
        .map(|v| v == "yes")
        .unwrap_or(false);
    let compose_available = flags.get("COMPOSE").map(|v| v == "yes").unwrap_or(false);
    let server_jar = artifact_from_flags(&flags, "SERVER_JAR");
    let gateway_classes = artifact_from_flags(&flags, "GATEWAY_CLASSES");
    let gateway_lib = artifact_from_flags(&flags, "GATEWAY_LIB");
    let frontend_dist = artifact_from_flags(&flags, "FRONTEND_DIST");

    if containers.is_empty() {
        for (name, service_key) in
            crate::commands::profile::deployment_profile().container_entries()
        {
            containers.push(ContainerStatusItem {
                name,
                service_key,
                state: "not_created".to_string(),
                health: "none".to_string(),
                status_text: "容器未创建".to_string(),
                ports: String::new(),
                image: String::new(),
            });
        }
    }

    let running_containers = containers
        .iter()
        .filter(|container| container.state == "running")
        .count();
    let total_containers = containers.len();
    let deployed = deploy_exists && !compose_file.is_empty();

    let mut issues = Vec::new();
    if !deployed {
        issues.push("未检测到部署目录或 compose 文件".to_string());
    }
    if !docker_available {
        issues.push("远程主机未安装 Docker".to_string());
    } else if !docker_running {
        issues.push("Docker 守护进程未运行".to_string());
    }
    if !compose_available {
        issues.push("docker compose 不可用".to_string());
    }
    if deployed && !server_jar.exists {
        issues.push("未找到后端 fat jar".to_string());
    }
    if deployed && !gateway_classes.exists {
        issues.push("未找到网关 classes".to_string());
    }
    if deployed && !frontend_dist.exists {
        issues.push("未找到前端产物".to_string());
    }
    if deployed && running_containers < total_containers {
        issues.push(format!(
            "{} / {} 个容器正在运行",
            running_containers, total_containers
        ));
    }

    let message = if deployed {
        format!(
            "已检测到部署目录 {}，{} / {} 个容器正在运行。",
            deploy_dir, running_containers, total_containers
        )
    } else {
        format!("未在 {} 检测到已部署服务。", deploy_dir)
    };

    Ok(RemoteDeploymentStatus {
        deployed,
        install_dir: install_dir.to_string(),
        deploy_dir,
        compose_file,
        docker_available,
        docker_running,
        compose_available,
        server_jar,
        gateway_classes,
        gateway_lib,
        frontend_dist,
        containers,
        running_containers,
        total_containers,
        issues,
        message,
    })
}

fn artifact_from_flags(flags: &HashMap<String, String>, key: &str) -> RemoteArtifactInfo {
    RemoteArtifactInfo {
        exists: flags
            .get(&format!("ART_{key}_EXISTS"))
            .map(|value| value == "yes")
            .unwrap_or(false),
        size_bytes: flags
            .get(&format!("ART_{key}_SIZE"))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0),
        modified: flags
            .get(&format!("ART_{key}_MTIME"))
            .cloned()
            .unwrap_or_default(),
    }
}

pub(super) fn remote_free_kb(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install_dir: &str,
) -> Result<u64, String> {
    let script = format!(
        "df -Pk {} | awk 'NR==2{{print $4}}'",
        shell_quote(install_dir)
    );
    let output = run_ssh_script(host, port, user, key, &script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    value
        .parse::<u64>()
        .map_err(|_| format!("无法读取目标机剩余磁盘空间: {value}"))
}
