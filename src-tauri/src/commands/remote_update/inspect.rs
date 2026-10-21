//! 目标机部署状态：容器、Compose 清单与交付包声明的产物。
//!
//! 产物清单来自当前项目的缓存交付包；本机没有缓存时回退到目标机的部署记录，
//! 不让界面伪造任何产物信息。
use super::bundle::normalize_install_dir;
use super::models::{RemoteArtifactStatus, RemoteDeploymentStatus};
use crate::commands::bundle::{Bundle, DeployedRecord, DEPLOYED_RECORD_DIR, DEPLOYED_RECORD_FILE};
use crate::commands::docker_ops::ContainerStatusItem;
use crate::commands::remote_ssh::{
    explain_ssh_failure, resolve_existing_key, run_ssh_script, shell_quote,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// 单个待探活的产物：描述来自交付包或目标机部署记录。
#[derive(Debug, Clone)]
struct ArtifactProbe {
    key: String,
    label: String,
    root: String,
    path: String,
    absolute: PathBuf,
}

fn deploy_root(install_dir: &Path, bundle: Option<&Bundle>) -> PathBuf {
    match bundle {
        Some(bundle) => install_dir.join(bundle.deploy_source_rel()),
        None => install_dir.join("deploy"),
    }
}

fn probes_from_bundle(bundle: &Bundle, install_dir: &Path) -> Vec<ArtifactProbe> {
    let deploy = deploy_root(install_dir, Some(bundle));
    bundle
        .descriptor
        .artifacts
        .iter()
        .map(|artifact| {
            let base = match artifact.root {
                crate::commands::bundle::ArtifactRoot::Install => install_dir.to_path_buf(),
                crate::commands::bundle::ArtifactRoot::Deploy => deploy.clone(),
            };
            ArtifactProbe {
                key: artifact.key.clone(),
                label: if artifact.label.trim().is_empty() {
                    artifact.key.clone()
                } else {
                    artifact.label.clone()
                },
                root: artifact.root.as_str().to_string(),
                path: artifact.path.clone(),
                absolute: base.join(artifact.path.trim()),
            }
        })
        .collect()
}

fn probes_from_record(record: &DeployedRecord, install_dir: &Path) -> Vec<ArtifactProbe> {
    let deploy = deploy_root(install_dir, None);
    record
        .artifacts
        .iter()
        .map(|artifact| {
            let base = if artifact.root == "deploy" {
                deploy.clone()
            } else {
                install_dir.to_path_buf()
            };
            ArtifactProbe {
                key: artifact.key.clone(),
                label: artifact.key.clone(),
                root: artifact.root.clone(),
                path: artifact.path.clone(),
                absolute: base.join(artifact.path.trim()),
            }
        })
        .collect()
}

/// 目标机上的 Compose 清单与部署根目录；本机没有缓存交付包时返回 None。
fn compose_and_deploy(install_dir: &Path, bundle: Option<&Bundle>) -> Option<(PathBuf, PathBuf)> {
    let bundle = bundle?;
    let deploy = deploy_root(install_dir, Some(bundle));
    let source = bundle.effective_compose();
    let compose = bundle
        .artifact_for_source(source)
        .map(|artifact| deploy.join(artifact.path.trim()))
        .unwrap_or_else(|| deploy.join(source));
    Some((compose, deploy))
}

fn deployment_status_script(
    install_dir: &Path,
    compose_and_deploy: Option<(PathBuf, PathBuf)>,
    probes: &[ArtifactProbe],
) -> String {
    let install = install_dir.to_string_lossy().to_string();
    let head = match compose_and_deploy {
        Some((compose, deploy)) => format!(
            "COMPOSE_FILE={compose}\nDEPLOY={deploy}\n",
            compose = shell_quote(&compose.to_string_lossy()),
            deploy = shell_quote(&deploy.to_string_lossy())
        ),
        None => crate::commands::docker_ops::compose_location_script(),
    };

    let mut script = format!(
        r#"set -e
INSTALL={install}
{head}echo "INSTALL_DIR=$INSTALL"
echo "DEPLOY_DIR=$DEPLOY"
if [ -d "$DEPLOY" ]; then echo "DEPLOY_EXISTS=yes"; else echo "DEPLOY_EXISTS=no"; fi
if [ -n "$COMPOSE_FILE" ] && [ -f "$COMPOSE_FILE" ]; then
  printf 'COMPOSE_FILE=%s\n' "$(basename "$COMPOSE_FILE")"
else
  echo "COMPOSE_FILE="
fi
if command -v docker >/dev/null 2>&1; then echo "DOCKER=yes"; else echo "DOCKER=no"; fi
if docker info >/dev/null 2>&1; then echo "DOCKER_RUNNING=yes"; else echo "DOCKER_RUNNING=no"; fi
if docker compose version >/dev/null 2>&1; then echo "COMPOSE=yes"; elif command -v docker-compose >/dev/null 2>&1; then echo "COMPOSE=yes"; else echo "COMPOSE=no"; fi
artifact() {{
  local idx=$1
  local path=$2
  if [ -e "$path" ]; then
    echo "ART_${{idx}}_EXISTS=yes"
    if [ -f "$path" ]; then
      echo "ART_${{idx}}_SIZE=$(stat -c '%s' "$path" 2>/dev/null || stat -f '%z' "$path" 2>/dev/null || echo 0)"
    else
      echo "ART_${{idx}}_SIZE=0"
    fi
    echo "ART_${{idx}}_MTIME=$(stat -c '%y' "$path" 2>/dev/null | cut -d. -f1)"
  else
    echo "ART_${{idx}}_EXISTS=no"
    echo "ART_${{idx}}_SIZE=0"
    echo "ART_${{idx}}_MTIME="
  fi
}}
"#,
        install = shell_quote(&install),
        head = head
    );

    for (index, probe) in probes.iter().enumerate() {
        script.push_str(&format!(
            "artifact {index} {}\n",
            shell_quote(&probe.absolute.to_string_lossy())
        ));
    }

    script.push_str(&format!(
        r#"if [ -f {record} ]; then
  echo "RECORD_BEGIN"
  cat {record}
  echo "RECORD_END"
fi
echo "---CONTAINERS---"
"#,
        record = shell_quote(
            &install_dir
                .join(DEPLOYED_RECORD_DIR)
                .join(DEPLOYED_RECORD_FILE)
                .to_string_lossy()
        )
    ));

    script
}

fn container_probe(container: &str) -> String {
    format!(
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
        container = shell_quote(container)
    )
}

fn parse_record(raw: &str) -> Option<DeployedRecord> {
    let start = raw.find("RECORD_BEGIN")?;
    let rest = &raw[start + "RECORD_BEGIN".len()..];
    let end = rest.find("RECORD_END")?;
    serde_json::from_str::<DeployedRecord>(rest[..end].trim()).ok()
}

fn probe_status(raw: &str, probes: &[ArtifactProbe]) -> Vec<RemoteArtifactStatus> {
    let mut flags: HashMap<String, String> = HashMap::new();
    for line in raw.lines() {
        if let Some((key, value)) = line.split_once('=') {
            if key.starts_with("ART_") {
                flags.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }
    probes
        .iter()
        .enumerate()
        .map(|(index, probe)| {
            let pick = |field: &str| {
                flags
                    .get(&format!("ART_{index}_{field}"))
                    .cloned()
                    .unwrap_or_default()
            };
            RemoteArtifactStatus {
                key: probe.key.clone(),
                label: probe.label.clone(),
                root: probe.root.clone(),
                path: probe.path.clone(),
                exists: pick("EXISTS") == "yes",
                size_bytes: pick("SIZE").parse::<u64>().unwrap_or(0),
                modified: pick("MTIME"),
            }
        })
        .collect()
}

fn stat_script(probes: &[ArtifactProbe]) -> String {
    let mut script = String::from("set -e\n");
    for (index, probe) in probes.iter().enumerate() {
        let path = shell_quote(&probe.absolute.to_string_lossy());
        script.push_str(&format!(
            r#"if [ -e {path} ]; then
  echo "ART_{index}_EXISTS=yes"
  if [ -f {path} ]; then echo "ART_{index}_SIZE=$(stat -c '%s' {path} 2>/dev/null || stat -f '%z' {path} 2>/dev/null || echo 0)"; else echo "ART_{index}_SIZE=0"; fi
  echo "ART_{index}_MTIME=$(stat -c '%y' {path} 2>/dev/null | cut -d. -f1)"
else
  echo "ART_{index}_EXISTS=no"; echo "ART_{index}_SIZE=0"; echo "ART_{index}_MTIME="
fi
"#,
        ))
    }
    script
}

pub(super) fn inspect_remote_deployment_inner(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install_dir: &str,
) -> Result<RemoteDeploymentStatus, String> {
    let install_path = PathBuf::from(install_dir);
    let bundle = crate::commands::projects::active_bundle();
    let bundle_ref = bundle.as_deref();

    let probes = match bundle_ref {
        Some(bundle) => probes_from_bundle(bundle, &install_path),
        None => Vec::new(),
    };
    let mut script = deployment_status_script(
        &install_path,
        compose_and_deploy(&install_path, bundle_ref),
        &probes,
    );
    if let Some(bundle) = bundle_ref {
        for service in &bundle.descriptor.runtime.services {
            script.push_str(&container_probe(&bundle.container_name(&service.key)));
        }
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
    let container_body = raw
        .split_once("---CONTAINERS---")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    for line in raw.lines() {
        if line.starts_with("CONTAINER|") {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            flags.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    for line in container_body.lines() {
        let Some(rest) = line.strip_prefix("CONTAINER|") else {
            continue;
        };
        let parts: Vec<&str> = rest.splitn(6, '|').collect();
        if parts.len() != 6 {
            continue;
        }
        let name = parts[0].trim().to_string();
        let state = parts[1].trim().to_string();
        let health = parts[2].trim().to_string();
        let image = parts[3].trim().to_string();
        let status_text = parts[4].trim().to_string();
        let ports = parts[5].trim().to_string();
        let service_key = match bundle_ref {
            Some(bundle) => bundle
                .descriptor
                .runtime
                .services
                .iter()
                .find(|service| bundle.container_name(&service.key) == name)
                .map(|service| service.key.clone())
                .unwrap_or_else(|| name.clone()),
            None => name.clone(),
        };
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

    let record = parse_record(&raw);
    let artifacts = if probes.is_empty() {
        match record.as_ref() {
            Some(record) => {
                let record_probes = probes_from_record(record, &install_path);
                let output = run_ssh_script(host, port, user, key, &stat_script(&record_probes))
                    .map_err(|e| format!("连接目标机失败: {e}"))?;
                if output.status.success() {
                    probe_status(&String::from_utf8_lossy(&output.stdout), &record_probes)
                } else {
                    Vec::new()
                }
            }
            None => Vec::new(),
        }
    } else {
        probe_status(&raw, &probes)
    };

    let deploy_dir = flags
        .get("DEPLOY_DIR")
        .cloned()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| deploy_root(&install_path, bundle_ref).to_string_lossy().to_string());
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

    if containers.is_empty() {
        if let Some(bundle) = bundle_ref {
            for service in &bundle.descriptor.runtime.services {
                containers.push(ContainerStatusItem {
                    name: bundle.container_name(&service.key),
                    service_key: service.key.clone(),
                    state: "not_created".to_string(),
                    health: "none".to_string(),
                    status_text: "容器未创建".to_string(),
                    ports: String::new(),
                    image: String::new(),
                });
            }
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
    if deployed {
        for artifact in &artifacts {
            if !artifact.exists {
                issues.push(format!("未找到 {}", artifact.path));
            }
        }
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
        artifacts,
        record_version: record
            .as_ref()
            .map(|record| record.version.clone())
            .unwrap_or_default(),
        record_missing: record.is_none(),
        containers,
        running_containers,
        total_containers,
        issues,
        message,
    })
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::commands::bundle::{ArtifactKind, ArtifactRoot};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "deploy-remote-inspect-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn remote_status_probes_descriptor_paths_in_both_layouts() {
        for layout in ["", "deploy/"] {
            let root = temp_root("status");
            fs::create_dir_all(root.join("bin")).unwrap();
            fs::create_dir_all(root.join(&format!("{layout}site"))).unwrap();
            fs::write(
                root.join(&format!("{layout}docker-compose.yml")),
                "services: {}\n",
            )
            .unwrap();
            let binary = root.join("bin/docker");
            fs::write(
                &binary,
                "#!/bin/sh\ncase \"$1\" in info|compose) exit 0;; *) exit 1;; esac\n",
            )
            .unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let mut paths = vec![root.join("bin")];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));

            let deploy = root.join(layout.trim_end_matches('/'));
            let probe = ArtifactProbe {
                key: "site".to_string(),
                label: "站点".to_string(),
                root: "deploy".to_string(),
                path: "site".to_string(),
                absolute: deploy.join("site"),
            };
            let compose = deploy.join("docker-compose.yml");
            let script = deployment_status_script(&root, Some((compose, deploy.clone())), &[probe]);
            let output = Command::new("sh")
                .args(["-c", &script])
                .env("PATH", std::env::join_paths(paths).unwrap())
                .output()
                .unwrap();
            assert!(output.status.success());
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text
                .lines()
                .any(|line| line == format!("DEPLOY_DIR={}", deploy.display())));
            assert!(text.lines().any(|line| line == "COMPOSE_FILE=docker-compose.yml"));
            assert!(text.lines().any(|line| line == "ART_0_EXISTS=yes"));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn artifact_probes_keep_descriptor_order() {
        let install = PathBuf::from("/srv/demo");
        let deploy = install.join("deploy");
        let root = temp_root("probes");
        let content = "payload";
        fs::create_dir_all(root.join("app")).unwrap();
        fs::write(root.join("app/site"), content).unwrap();
        let manifest = serde_json::json!({
            "apiVersion": "deploy.desktop/bundle/v2",
            "kind": "DeploymentBundle",
            "metadata": { "product": "demo", "version": "1", "commit": "", "commitShort": "" },
            "runtime": { "type": "compose", "compose": "deploy/docker-compose.yml", "containerPrefix": "demo", "services": [] },
            "components": [],
            "artifacts": [
                { "key": "site", "source": "app/site", "root": "deploy", "path": "site", "type": "file" },
                { "key": "compose", "source": "deploy/docker-compose.yml", "root": "deploy",
                  "path": "docker-compose.yml", "type": "file" }
            ],
            "files": [
                { "path": "app/site", "size": content.len(), "sha256": crate::commands::bundle::sha256_hex(content.as_bytes()) },
                { "path": "deploy/docker-compose.yml", "size": 5, "sha256": crate::commands::bundle::sha256_hex(b"a: 1\n") }
            ],
        });
        fs::create_dir_all(root.join("deploy")).unwrap();
        fs::write(root.join("deploy/docker-compose.yml"), "a: 1\n").unwrap();
        fs::write(root.join("bundle.json"), serde_json::to_string(&manifest).unwrap()).unwrap();
        let _ = &deploy;
        let bundle = Bundle::load(&root).unwrap();
        let probes = probes_from_bundle(&bundle, &install);
        assert_eq!(probes.len(), 2);
        assert_eq!(probes[0].key, "site");
        assert_eq!(probes[0].absolute, install.join("deploy/site"));
        assert_eq!(probes[0].root, ArtifactRoot::Deploy.as_str());
        assert_eq!(ArtifactKind::File.as_str(), "file");
        fs::remove_dir_all(root).unwrap();
    }
}
