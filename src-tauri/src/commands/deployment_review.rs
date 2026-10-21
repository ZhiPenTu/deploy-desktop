use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::remote_ssh::{find_binary, resolve_existing_key, run_ssh_script, shell_quote};
use super::remote_update::{
    compose_service_views, match_tarball, normalize_image_ref, resolve_bundle_file,
    scan_image_files, ComposeServiceView,
};

const COMPOSE_LIMIT: usize = 1024 * 1024;
const CONTAINER_FORMAT: &str = "{{.Name}}\t{{.Config.Image}}\t{{.State.Status}}\t{{index .Config.Labels \"com.docker.compose.service\"}}\t{{index .Config.Labels \"com.docker.compose.project.working_dir\"}}\t{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentServiceReview {
    pub name: String,
    pub image: String,
    pub current_image: String,
    pub container_name: String,
    pub container_state: String,
    pub container_health: String,
    pub ports: Vec<String>,
    pub depends_on: Vec<String>,
    pub profiles: Vec<String>,
    /// added / changed / unchanged / removed; changes describe Compose declarations.
    pub change: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentImageReview {
    pub image_ref: String,
    pub services: Vec<String>,
    pub docker_present: bool,
    /// ready / missing / unconfirmed. A tarball alone never makes an image ready.
    pub status: String,
    pub local_file: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentReview {
    pub compose_file: String,
    pub target_compose_file: String,
    // Read-only source snapshots, never substituted for the execution configuration.
    pub current_compose: String,
    pub desired_compose: String,
    pub install_dir: String,
    pub has_deployment: bool,
    pub target_checked: bool,
    pub image_check: String,
    pub docker_running: bool,
    pub compose_available: bool,
    pub compose_changed: bool,
    pub services: Vec<DeploymentServiceReview>,
    pub images: Vec<DeploymentImageReview>,
    pub missing_images: Vec<String>,
    pub issues: Vec<String>,
    pub ready: bool,
    pub can_first_deploy: bool,
    pub can_incremental_update: bool,
}

/// Read-only comparison. No images are pulled, no files are uploaded and no containers are changed.
#[tauri::command]
pub async fn inspect_deployment_review(
    bundle_dir: String,
    install_dir: String,
    image_dir: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<DeploymentReview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        review_blocking(
            bundle_dir,
            install_dir,
            image_dir,
            host,
            port,
            user,
            key_path,
        )
    })
    .await
    .map_err(|_| "核对任务未能完成，请重试".to_string())?
}

fn review_blocking(
    bundle_dir: String,
    install_dir: String,
    image_dir: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<DeploymentReview, String> {
    let root = fs::canonicalize(bundle_dir.trim())
        .map_err(|_| "无法访问交付包目录，请重新选择并解压制品".to_string())?;
    let compose_path = ["deploy/docker-compose.lan.yml", "deploy/docker-compose.yml"]
        .iter()
        .find(|name| root.join(name).is_file())
        .ok_or_else(|| "交付包缺少 Docker Compose 清单，请重新选择制品".to_string())?;
    let path = resolve_bundle_file(&root, compose_path)?;
    let desired = read_compose(&path)?;
    let services = parse_services(&desired)?;
    if services.is_empty() {
        return Err("交付包的 Docker Compose 清单没有服务，请重新选择制品".to_string());
    }

    let install = install_dir.trim().trim_end_matches('/');
    if install.is_empty() || install.contains(['\n', '\r', '\0']) {
        return Err("请填写有效的目标部署目录".to_string());
    }
    let images = unique_images(&services);
    let remote = host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty());
    let target = if let Some(host) = remote {
        let user = user
            .as_deref()
            .map(str::trim)
            .filter(|user| !user.is_empty())
            .ok_or_else(|| "请选择具有 SSH 登录账号的主机".to_string())?;
        let key = resolve_existing_key(key_path, host)?;
        let output = run_ssh_script(
            host,
            port.unwrap_or(22),
            user,
            key.as_deref(),
            &target_script(install, &images),
        )
        .map_err(|_| "无法连接目标主机，请检查主机连接和 SSH 凭证后重试".to_string())?;
        if !output.status.success() {
            return Err("无法完成目标主机核对，请检查 SSH 连接与部署目录的读取权限".to_string());
        }
        parse_target_output(&String::from_utf8_lossy(&output.stdout))?
    } else {
        inspect_local_target(Path::new(install), &images)?
    };
    let mut review = compare(*compose_path, install, &desired, &services, &images, target)?;
    review.image_check = if review.target_checked && review.docker_running {
        if remote.is_some() {
            "remote"
        } else {
            "local"
        }
    } else {
        "unavailable"
    }
    .to_string();

    // Package discovery is only a preparation hint. Docker readiness above is always checked
    // with the complete requested image reference, including any sha256 digest.
    let media = image_dir
        .as_deref()
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.clone());
    let media = if media.join("images").is_dir() {
        media.join("images")
    } else {
        media
    };
    let files = scan_image_files(&media);
    let manifest = super::image_manager::load_image_manifest(&media);
    let profile = super::profile::deployment_profile();
    for image in &mut review.images {
        let (package, _) = match_tarball(
            &normalize_image_ref(&image.image_ref),
            &files,
            &manifest,
            &profile.images,
        );
        image.local_file = package.map(|(name, _)| name).unwrap_or_default();
    }
    Ok(review)
}

fn read_compose(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path)
        .map_err(|_| "无法读取 Docker Compose 清单，请检查文件权限".to_string())?;
    if metadata.len() > COMPOSE_LIMIT as u64 {
        return Err("Docker Compose 清单超过 1 MB，无法完成核对".to_string());
    }
    fs::read_to_string(path)
        .map_err(|_| "无法读取 Docker Compose 清单，请确认文件使用 UTF-8 编码".to_string())
}

fn parse_services(content: &str) -> Result<Vec<ComposeServiceView>, String> {
    // Do not return parser excerpts: a YAML document may contain credentials.
    compose_service_views(content)
        .map_err(|_| "Docker Compose 清单格式有误，请检查清单后重试".to_string())
}

fn unique_images(services: &[ComposeServiceView]) -> Vec<String> {
    let mut images = Vec::new();
    for service in services {
        if !service.image.is_empty() && !images.contains(&service.image) {
            images.push(service.image.clone());
        }
    }
    images
}

fn inspect_local_target(install: &Path, images: &[String]) -> Result<TargetSnapshot, String> {
    check_local_endpoint()?;
    let mut target = local_compose_snapshot(install)?;
    target.docker_running = Command::new(find_binary("docker"))
        .arg("info")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    target.compose_available = Command::new(find_binary("docker"))
        .args(["compose", "version"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if target.docker_running {
        let containers = Command::new(find_binary("docker"))
            .args(["ps", "-aq"])
            .output()
            .map_err(|_| "无法读取本机容器状态，请检查 Docker 后重试".to_string())?;
        if !containers.status.success() {
            return Err("无法读取本机容器状态，请检查 Docker 后重试".to_string());
        }
        let ids = String::from_utf8_lossy(&containers.stdout);
        let ids: Vec<&str> = ids
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .collect();
        if !ids.is_empty() {
            let output = Command::new(find_binary("docker"))
                .args(["inspect", "--format", CONTAINER_FORMAT, "--"])
                .args(&ids)
                .output()
                .map_err(|_| "无法核对本机容器状态，请重试".to_string())?;
            if !output.status.success() {
                return Err("无法核对本机容器状态，请重试".to_string());
            }
            target.containers = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| {
                    parse_container(line)
                        .ok_or_else(|| "容器状态核对结果不完整，请检查 Docker 后重试".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if target.containers.len() != ids.len() {
                return Err("容器状态核对结果不完整，请检查 Docker 后重试".to_string());
            }
        }
        target.containers_checked = true;
    }
    for (index, reference) in images.iter().enumerate() {
        let state = if !target.docker_running || !valid_image_reference(reference) {
            "unconfirmed"
        } else {
            match Command::new(find_binary("docker"))
                .args(["image", "inspect", "--format", "{{.Id}}", "--", reference])
                .output()
            {
                Ok(output)
                    if output.status.success()
                        && valid_image_id(String::from_utf8_lossy(&output.stdout).trim()) =>
                {
                    "ready"
                }
                Ok(output) if output.status.success() => "unconfirmed",
                Ok(output)
                    if String::from_utf8_lossy(&output.stderr)
                        .to_ascii_lowercase()
                        .contains("no such image") =>
                {
                    "missing"
                }
                Ok(_) => "unconfirmed",
                Err(_) => "unconfirmed",
            }
        };
        target.image_states.insert(index, state.to_string());
    }
    Ok(target)
}

fn local_compose_snapshot(install: &Path) -> Result<TargetSnapshot, String> {
    let mut target = TargetSnapshot {
        checked: true,
        ..TargetSnapshot::default()
    };
    for name in [
        "docker-compose.lan.yml",
        "docker-compose.yml",
        "deploy/docker-compose.lan.yml",
        "deploy/docker-compose.yml",
    ] {
        let path = install.join(name);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {
                target.compose = read_compose(&path)?;
                target.compose_file = path.to_string_lossy().to_string();
                target.compose_working_dir = fs::canonicalize(path.parent().unwrap_or(install))
                    .map_err(|_| "无法访问本机部署目录，请检查目录权限".to_string())?
                    .to_string_lossy()
                    .to_string();
                break;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("无法读取本机部署目录，请检查目录权限后重试".to_string()),
        }
    }
    Ok(target)
}

fn check_local_endpoint() -> Result<(), String> {
    let context = Command::new(find_binary("docker"))
        .args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ])
        .output()
        .map_err(|_| "无法读取 Docker 连接，请确认已安装 Docker".to_string())?;
    if !context.status.success() {
        return Err("无法读取 Docker 连接，请检查本机 Docker 配置后重试".to_string());
    }
    let context = String::from_utf8_lossy(&context.stdout);
    let host_override = std::env::var("DOCKER_HOST").ok();
    let context_override = std::env::var("DOCKER_CONTEXT").ok();
    if !local_endpoint(
        &context,
        host_override.as_deref(),
        context_override.as_deref(),
    ) {
        return Err("本机部署需要连接本机 Docker，请先切换 Docker 连接后重新核对".to_string());
    }
    Ok(())
}

fn local_endpoint(
    context: &str,
    host_override: Option<&str>,
    context_override: Option<&str>,
) -> bool {
    let endpoint = if context_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        context
    } else {
        host_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(context)
    }
    .trim();
    endpoint.starts_with("unix://") || endpoint.starts_with("npipe://")
}

fn valid_image_id(reference: &str) -> bool {
    reference
        .strip_prefix("sha256:")
        .map(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .unwrap_or(false)
}

fn valid_image_reference(reference: &str) -> bool {
    if reference.is_empty()
        || reference.len() > 512
        || !reference.is_ascii()
        || reference.starts_with(['-', '/', '.'])
        || reference
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || reference.contains("://")
        || reference.contains(['$', '\\', '\'', '"', '`', ';', '|', '&', '(', ')', '{', '}'])
    {
        return false;
    }
    let name = if let Some((name, digest)) = reference.split_once('@') {
        if !valid_image_id(digest) || name.is_empty() || name.contains('@') {
            return false;
        }
        name
    } else {
        reference
    };
    let last_part = name.rsplit('/').next().unwrap_or_default();
    let repository = if let Some((_, tag)) = last_part.rsplit_once(':') {
        if tag.is_empty()
            || tag.len() > 128
            || tag.starts_with(['.', '-'])
            || !tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            return false;
        }
        &name[..name.len() - tag.len() - 1]
    } else {
        name
    };
    let segments: Vec<&str> = repository.split('/').collect();
    segments.iter().enumerate().all(|(index, segment)| {
        if segment.is_empty() {
            return false;
        }
        if index == 0
            && segments.len() > 1
            && (segment.contains('.') || segment.contains(':') || *segment == "localhost")
        {
            return segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b));
        }
        let bytes = segment.as_bytes();
        bytes
            .first()
            .map(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            .unwrap_or(false)
            && bytes
                .last()
                .map(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                .unwrap_or(false)
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(b))
            && !segment.contains("..")
    })
}

fn target_script(install: &str, images: &[String]) -> String {
    let mut script = format!(
        r#"set -eu
INSTALL={install}
DEPLOY="$INSTALL/deploy"
if [ -d "$INSTALL" ] && [ ! -r "$INSTALL" ]; then exit 3; fi
if [ -d "$DEPLOY" ] && [ ! -r "$DEPLOY" ]; then exit 3; fi
COMPOSE_FILE=""
if [ -f "$INSTALL/docker-compose.lan.yml" ]; then
  COMPOSE_FILE="$INSTALL/docker-compose.lan.yml"
elif [ -f "$INSTALL/docker-compose.yml" ]; then
  COMPOSE_FILE="$INSTALL/docker-compose.yml"
elif [ -f "$DEPLOY/docker-compose.lan.yml" ]; then
  COMPOSE_FILE="$DEPLOY/docker-compose.lan.yml"
elif [ -f "$DEPLOY/docker-compose.yml" ]; then
  COMPOSE_FILE="$DEPLOY/docker-compose.yml"
fi
printf 'TARGET_CHECKED=yes\nCOMPOSE_FILE=%s\n' "$COMPOSE_FILE"
if docker info >/dev/null 2>&1; then
  echo 'DOCKER_RUNNING=yes'
else
  echo 'DOCKER_RUNNING=no'
fi
if docker compose version >/dev/null 2>&1; then
  echo 'COMPOSE_AVAILABLE=yes'
else
  echo 'COMPOSE_AVAILABLE=no'
fi
if [ -n "$COMPOSE_FILE" ]; then
  printf 'COMPOSE_WORKING_DIR=%s\n' "$(cd "$(dirname "$COMPOSE_FILE")" && pwd -P)"
  [ -r "$COMPOSE_FILE" ] || exit 3
  size=$(wc -c < "$COMPOSE_FILE")
  [ "$size" -le {limit} ] || exit 4
  echo '__REVIEW_COMPOSE_BEGIN__'
  cat "$COMPOSE_FILE"
  printf '\n__REVIEW_COMPOSE_END__\n'
fi
echo '__REVIEW_CONTAINERS_BEGIN__'
if docker info >/dev/null 2>&1; then
  containers=$(docker ps -aq 2>/dev/null) || exit 5
  if [ -n "$containers" ]; then
    docker inspect --format {container_format} -- $containers 2>/dev/null || exit 5
  fi
fi
echo '__REVIEW_CONTAINERS_END__'
echo 'CONTAINERS_CHECKED=yes'
"#,
        install = shell_quote(install),
        limit = COMPOSE_LIMIT,
        container_format = shell_quote(CONTAINER_FORMAT)
    );
    for (index, reference) in images.iter().enumerate() {
        if !valid_image_reference(reference) {
            script.push_str(&format!("echo 'IMAGE|{index}|unconfirmed'\n"));
            continue;
        }
        script.push_str(&format!(
            "if docker info >/dev/null 2>&1; then\n  if image_id=$(docker image inspect --format '{{{{.Id}}}}' -- {reference} 2>&1); then printf 'IMAGE|{index}|ready|%s\\n' \"$image_id\"; else\n    case \"$image_id\" in *[Nn]o\\ such\\ image*) echo 'IMAGE|{index}|missing';; *) echo 'IMAGE|{index}|unconfirmed';; esac\n  fi\nelse\n  echo 'IMAGE|{index}|unconfirmed'\nfi\n",
            reference = shell_quote(reference),
        ));
    }
    script
}

#[derive(Debug, Default)]
struct TargetSnapshot {
    checked: bool,
    docker_running: bool,
    compose_available: bool,
    compose_file: String,
    compose_working_dir: String,
    compose: String,
    containers: Vec<TargetContainer>,
    containers_checked: bool,
    image_states: HashMap<usize, String>,
}

#[derive(Debug)]
struct TargetContainer {
    name: String,
    image: String,
    state: String,
    health: String,
    service: String,
    working_dir: String,
}

fn parse_target_output(output: &str) -> Result<TargetSnapshot, String> {
    let mut target = TargetSnapshot::default();
    let mut section = "flags";
    for line in output.lines() {
        match line {
            "__REVIEW_COMPOSE_BEGIN__" => {
                section = "compose";
                continue;
            }
            "__REVIEW_COMPOSE_END__" => {
                section = "flags";
                continue;
            }
            "__REVIEW_CONTAINERS_BEGIN__" => {
                section = "containers";
                continue;
            }
            "__REVIEW_CONTAINERS_END__" => {
                section = "flags";
                continue;
            }
            _ => {}
        }
        if section == "compose" {
            target.compose.push_str(line);
            target.compose.push('\n');
        } else if section == "containers" {
            target.containers.push(
                parse_container(line)
                    .ok_or_else(|| "容器状态核对结果不完整，请检查目标主机后重试".to_string())?,
            );
        } else if let Some(rest) = line.strip_prefix("IMAGE|") {
            let fields: Vec<&str> = rest.split('|').collect();
            if fields.len() >= 2 {
                if let Ok(index) = fields[0].parse() {
                    let state = match fields[1] {
                        "ready" if fields.len() == 3 && valid_image_id(fields[2]) => "ready",
                        "missing" => "missing",
                        _ => "unconfirmed",
                    };
                    target.image_states.insert(index, state.to_string());
                }
            }
        } else if let Some((key, value)) = line.split_once('=') {
            match key {
                "TARGET_CHECKED" => target.checked = value == "yes",
                "DOCKER_RUNNING" => target.docker_running = value == "yes",
                "COMPOSE_AVAILABLE" => target.compose_available = value == "yes",
                "COMPOSE_FILE" => target.compose_file = value.to_string(),
                "COMPOSE_WORKING_DIR" => target.compose_working_dir = value.to_string(),
                "CONTAINERS_CHECKED" => target.containers_checked = value == "yes",
                _ => {}
            }
        }
    }
    if !target.checked
        || section != "flags"
        || (target.docker_running && !target.containers_checked)
    {
        return Err("目标主机未返回完整核对结果，请检查连接后重试".to_string());
    }
    Ok(target)
}

fn parse_container(line: &str) -> Option<TargetContainer> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 6
        || fields[0].trim_start_matches('/').is_empty()
        || !valid_image_reference(fields[1])
        || ![
            "created",
            "restarting",
            "running",
            "removing",
            "paused",
            "exited",
            "dead",
        ]
        .contains(&fields[2])
        || !["none", "healthy", "unhealthy", "starting"].contains(&fields[5])
    {
        return None;
    }
    Some(TargetContainer {
        name: fields[0].trim_start_matches('/').to_string(),
        image: fields[1].to_string(),
        state: fields[2].to_string(),
        service: fields[3].to_string(),
        working_dir: fields[4].to_string(),
        health: fields[5].to_string(),
    })
}

fn compare(
    compose_file: &str,
    install: &str,
    desired: &str,
    services: &[ComposeServiceView],
    images: &[String],
    target: TargetSnapshot,
) -> Result<DeploymentReview, String> {
    let has_deployment = !target.compose_file.is_empty();
    let current_services = if has_deployment {
        parse_services(&target.compose)
            .map_err(|_| "目标部署目录的 Docker Compose 清单无法解析，请检查现有部署".to_string())?
    } else {
        Vec::new()
    };
    let desired_doc: serde_yaml::Value = serde_yaml::from_str(desired)
        .map_err(|_| "交付包的 Docker Compose 清单无法解析".to_string())?;
    let current_doc: serde_yaml::Value = if has_deployment {
        serde_yaml::from_str(&target.compose)
            .map_err(|_| "目标 Docker Compose 清单无法解析".to_string())?
    } else {
        serde_yaml::Value::Null
    };
    let mut issues = Vec::new();
    if !target.docker_running {
        issues.push("目标主机的 Docker 不可用，请先完成环境检测并启动 Docker".to_string());
    }
    if !target.compose_available {
        issues.push("目标主机缺少 Docker Compose，请先准备运行环境".to_string());
    }
    if target.docker_running && !target.containers_checked {
        issues.push("容器状态未确认，请检查目标主机后重新核对".to_string());
    }
    let supported_update_layout = !has_deployment
        || Path::new(&target.compose_file)
            .parent()
            .map(|parent| same_directory(&parent.to_string_lossy(), &format!("{install}/deploy")))
            .unwrap_or(false);
    if !supported_update_layout {
        issues.push(
            "现有清单不在所选目录的 deploy/ 下，请选择包含 deploy/ 的项目根目录后重新核对"
                .to_string(),
        );
    }
    let compose_working_dir = if !target.compose_working_dir.is_empty() {
        target.compose_working_dir.clone()
    } else if has_deployment {
        Path::new(&target.compose_file)
            .parent()
            .unwrap_or_else(|| Path::new(install))
            .to_string_lossy()
            .to_string()
    } else {
        format!("{install}/deploy")
    };

    let mut service_reviews = Vec::new();
    for service in services {
        let old = current_services.iter().find(|old| old.name == service.name);
        let container = target.containers.iter().find(|container| {
            (!service.container_name.is_empty() && container.name == service.container_name)
                || (container.service == service.name
                    && same_directory(&container.working_dir, &compose_working_dir))
        });
        if let Some(container) = container {
            if !same_directory(&container.working_dir, &compose_working_dir) {
                issues.push(format!(
                    "容器 {} 已存在，但不属于所选部署目录，请检查目标主机与部署位置",
                    container.name
                ));
            }
        }
        let change = match old {
            None => "added",
            Some(_)
                if desired_doc
                    .get("services")
                    .and_then(|all| all.get(&service.name))
                    == current_doc
                        .get("services")
                        .and_then(|all| all.get(&service.name)) =>
            {
                "unchanged"
            }
            Some(_) => "changed",
        };
        if service.image.is_empty() {
            issues.push(format!(
                "服务 {} 没有可用镜像声明，请补齐交付包清单",
                service.name
            ));
        } else if service.image.contains('$') {
            issues.push(format!(
                "服务 {} 的镜像地址尚未确定，请检查交付包中的环境配置",
                service.name
            ));
        } else if !valid_image_reference(&service.image) {
            issues.push(format!(
                "服务 {} 的镜像地址无效，请检查交付包清单",
                service.name
            ));
        }
        service_reviews.push(DeploymentServiceReview {
            name: service.name.clone(),
            image: service.image.clone(),
            current_image: container
                .map(|c| c.image.clone())
                .or_else(|| old.map(|old| old.image.clone()))
                .unwrap_or_default(),
            container_name: container
                .map(|c| c.name.clone())
                .unwrap_or_else(|| service.container_name.clone()),
            container_state: if target.docker_running {
                container
                    .map(|c| c.state.clone())
                    .unwrap_or_else(|| "not_created".to_string())
            } else {
                "unconfirmed".to_string()
            },
            container_health: container
                .map(|c| c.health.clone())
                .unwrap_or_else(|| "none".to_string()),
            ports: service.ports.clone(),
            depends_on: service.depends_on.clone(),
            profiles: service.profiles.clone(),
            change: change.to_string(),
        });
    }
    for old in &current_services {
        if !services.iter().any(|service| service.name == old.name) {
            service_reviews.push(DeploymentServiceReview {
                name: old.name.clone(),
                image: String::new(),
                current_image: old.image.clone(),
                container_name: old.container_name.clone(),
                container_state: "unconfirmed".to_string(),
                container_health: "none".to_string(),
                ports: old.ports.clone(),
                depends_on: old.depends_on.clone(),
                profiles: old.profiles.clone(),
                change: "removed".to_string(),
            });
        }
    }
    let image_reviews: Vec<DeploymentImageReview> = images
        .iter()
        .enumerate()
        .map(|(index, reference)| {
            let state = target
                .image_states
                .get(&index)
                .map(String::as_str)
                .unwrap_or("unconfirmed");
            let status = if !valid_image_reference(reference) {
                "unconfirmed"
            } else if target.docker_running && state == "ready" {
                "ready"
            } else if target.docker_running && state == "missing" {
                "missing"
            } else {
                "unconfirmed"
            };
            DeploymentImageReview {
                image_ref: reference.clone(),
                services: services
                    .iter()
                    .filter(|s| s.image == *reference)
                    .map(|s| s.name.clone())
                    .collect(),
                docker_present: status == "ready",
                status: status.to_string(),
                local_file: String::new(),
            }
        })
        .collect();
    let missing_images: Vec<String> = image_reviews
        .iter()
        .filter(|image| image.status == "missing")
        .map(|image| image.image_ref.clone())
        .collect();
    if !missing_images.is_empty() {
        issues.push(format!(
            "目标主机缺少 {} 个镜像，请先导入镜像后重新核对",
            missing_images.len()
        ));
    }
    if image_reviews
        .iter()
        .any(|image| image.status == "unconfirmed")
    {
        issues.push("部分镜像状态未确认，请检查目标主机后重新核对".to_string());
    }
    let ready =
        target.checked && target.docker_running && target.compose_available && issues.is_empty();
    Ok(DeploymentReview {
        compose_file: compose_file.to_string(),
        target_compose_file: target.compose_file,
        current_compose: target.compose,
        desired_compose: desired.to_string(),
        install_dir: install.to_string(),
        has_deployment,
        target_checked: target.checked,
        image_check: "unavailable".to_string(),
        docker_running: target.docker_running,
        compose_available: target.compose_available,
        compose_changed: desired_doc != current_doc,
        services: service_reviews,
        images: image_reviews,
        missing_images,
        issues,
        ready,
        can_first_deploy: ready && !has_deployment,
        can_incremental_update: ready && has_deployment && supported_update_layout,
    })
}

fn same_directory(left: &str, right: &str) -> bool {
    left.replace('\\', "/").trim_end_matches('/') == right.replace('\\', "/").trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(compose: &str, statuses: &[(usize, &str)]) -> TargetSnapshot {
        TargetSnapshot {
            checked: true,
            docker_running: true,
            compose_available: true,
            containers_checked: true,
            compose_file: if compose.is_empty() {
                String::new()
            } else {
                "/srv/demo/deploy/docker-compose.yml".to_string()
            },
            compose: compose.to_string(),
            image_states: statuses
                .iter()
                .map(|(i, status)| (*i, status.to_string()))
                .collect(),
            ..TargetSnapshot::default()
        }
    }

    #[test]
    fn compares_service_additions_changes_removals_and_configuration() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n    ports: ['8081:80']\n  redis:\n    image: redis:7\n";
        let current = "services:\n  web:\n    image: nginx:1.27\n    ports: ['8080:80']\n  previous:\n    image: nginx:1.26\n";
        let services = parse_services(desired).unwrap();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &unique_images(&services),
            snapshot(current, &[(0, "ready"), (1, "missing")]),
        )
        .unwrap();
        assert_eq!(review.services[0].change, "changed");
        assert_eq!(review.services[1].change, "added");
        assert_eq!(review.services[2].change, "removed");
        assert_eq!(review.current_compose, current);
        assert_eq!(review.desired_compose, desired);
        let serialized = serde_json::to_value(&review).unwrap();
        assert_eq!(serialized["currentCompose"], current);
        assert_eq!(serialized["desiredCompose"], desired);
        assert!(review.has_deployment);
        assert_eq!(review.missing_images, ["redis:7"]);
        assert!(!review.ready);
        assert!(!review.can_incremental_update);
    }

    #[test]
    fn first_deployment_requires_confirmed_docker_and_every_image() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let images = unique_images(&services);
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &images,
            snapshot("", &[(0, "ready")]),
        )
        .unwrap();
        assert!(review.ready);
        assert!(review.can_first_deploy);
        assert!(!review.can_incremental_update);
        let unknown = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &images,
            snapshot("", &[]),
        )
        .unwrap();
        assert!(!unknown.ready);
        assert_eq!(unknown.images[0].status, "unconfirmed");
    }

    #[test]
    fn digest_reference_is_never_replaced_with_a_tag() {
        let digest = format!("registry.example.com/demo/web:1@sha256:{}", "a".repeat(64));
        let script = target_script("/srv/demo's directory", &[digest.clone()]);
        assert!(script.contains(&format!(
            "docker image inspect --format '{{{{.Id}}}}' -- '{}'",
            digest
        )));
        assert!(!script.contains("docker images"));
        assert!(script.contains("INSTALL='/srv/demo'\\''s directory'"));
    }

    #[test]
    fn parser_separates_compose_from_flags_and_container_metadata() {
        let raw = format!("TARGET_CHECKED=yes\nDOCKER_RUNNING=yes\nCOMPOSE_AVAILABLE=yes\nCOMPOSE_FILE=/srv/demo/deploy/docker-compose.yml\n__REVIEW_COMPOSE_BEGIN__\nservices:\n  web:\n    image: nginx:1.27\n__REVIEW_COMPOSE_END__\n__REVIEW_CONTAINERS_BEGIN__\n/demo-web-1\tnginx:1.26\trunning\tweb\t/srv/demo/deploy\thealthy\n__REVIEW_CONTAINERS_END__\nCONTAINERS_CHECKED=yes\nIMAGE|0|ready|sha256:{}\n", "a".repeat(64));
        let target = parse_target_output(&raw).unwrap();
        assert_eq!(target.containers.len(), 1);
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert_eq!(review.services[0].container_state, "running");
        assert_eq!(review.services[0].current_image, "nginx:1.26");
        assert_eq!(review.services[0].container_health, "healthy");
        assert!(review.ready);
        assert!(!review.can_first_deploy);
    }

    #[test]
    fn local_docker_endpoint_follows_context_and_host_precedence() {
        assert!(local_endpoint("unix:///var/run/docker.sock", None, None));
        assert!(local_endpoint("npipe:////./pipe/docker_engine", None, None));
        assert!(!local_endpoint("ssh://remote.example.com", None, None));
        assert!(!local_endpoint(
            "unix:///var/run/docker.sock",
            Some("tcp://remote.example.com:2376"),
            None
        ));
        assert!(local_endpoint(
            "unix:///var/run/docker.sock",
            Some("tcp://remote.example.com:2376"),
            Some("desktop-linux")
        ));
        assert!(!local_endpoint(
            "ssh://remote.example.com",
            Some("unix:///var/run/docker.sock"),
            Some("remote-context")
        ));
    }

    #[test]
    fn compose_preference_and_container_matching_follow_actual_root_location() {
        let root = std::env::temp_dir().join(format!(
            "deploy-review-root-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("deploy")).unwrap();
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        fs::write(root.join("docker-compose.yml"), desired).unwrap();
        fs::write(root.join("deploy/docker-compose.lan.yml"), "services: {}\n").unwrap();
        let mut target = local_compose_snapshot(&root).unwrap();
        assert_eq!(
            Path::new(&target.compose_file),
            super::super::docker_ops::resolve_compose_file(&root).unwrap()
        );
        target.checked = true;
        target.docker_running = true;
        target.compose_available = true;
        target.containers_checked = true;
        target.image_states.insert(0, "ready".to_string());
        target.containers.push(TargetContainer {
            name: "demo-web-1".to_string(),
            image: "nginx:1.27".to_string(),
            state: "running".to_string(),
            health: "healthy".to_string(),
            service: "web".to_string(),
            working_dir: target.compose_working_dir.clone(),
        });
        let services = parse_services(desired).unwrap();
        let review = compare(
            "deploy/docker-compose.yml",
            root.to_str().unwrap(),
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert!(!review.ready);
        assert!(!review.can_first_deploy);
        assert!(!review.can_incremental_update);
        assert_eq!(review.services[0].container_state, "running");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_containers_or_invalid_image_refs_cannot_be_ready() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot("", &[(0, "ready")]);
        target.containers_checked = false;
        assert!(
            !compare(
                "deploy/docker-compose.yml",
                "/srv/demo",
                desired,
                &services,
                &unique_images(&services),
                target
            )
            .unwrap()
            .ready
        );
        for reference in ["--help", "https://registry.example.com/web", "nginx image", "nginx@sha256:abc", "nginx;true", "${IMAGE}", "nginx@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@sha256:bbbb"] {
            assert!(!valid_image_reference(reference), "unexpected valid reference: {reference}");
        }
        let invalid = "services:\n  web:\n    image: '--help'\n";
        let services = parse_services(invalid).unwrap();
        assert!(
            !compare(
                "deploy/docker-compose.yml",
                "/srv/demo",
                invalid,
                &services,
                &unique_images(&services),
                snapshot("", &[(0, "ready")])
            )
            .unwrap()
            .ready
        );
        assert!(parse_target_output(
            "TARGET_CHECKED=yes\nDOCKER_RUNNING=yes\nCOMPOSE_AVAILABLE=yes\n"
        )
        .is_err());
    }

    #[test]
    fn compose_v1_alone_cannot_satisfy_execution_requirements() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot("", &[(0, "ready")]);
        target.compose_available = false;
        assert!(
            !compare(
                "deploy/docker-compose.yml",
                "/srv/demo",
                desired,
                &services,
                &unique_images(&services),
                target
            )
            .unwrap()
            .ready
        );
        assert!(!target_script("/srv/demo", &[]).contains("command -v docker-compose"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_probe_script_reads_root_compose_and_checks_complete_image_identity() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "deploy-review-probe-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("deploy")).unwrap();
        fs::write(
            root.join("docker-compose.yml"),
            "services:\n  web:\n    image: nginx:1.27\n",
        )
        .unwrap();
        fs::write(root.join("deploy/docker-compose.lan.yml"), "services: {}\n").unwrap();
        let binary = root.join("bin/docker");
        fs::write(
            &binary,
            r#"#!/bin/sh
case "$1" in
  info|compose) exit 0;;
  ps) printf 'container-id\n';;
  inspect) printf '/demo-web\tnginx:1.27\trunning\tweb\t%s\thealthy\n' "$REVIEW_TEST_ROOT";;
  image) printf 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n';;
  *) exit 1;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![root.join("bin")];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let output = Command::new("bash")
            .arg("-c")
            .arg(target_script(
                root.to_str().unwrap(),
                &["nginx:1.27".to_string()],
            ))
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("REVIEW_TEST_ROOT", fs::canonicalize(&root).unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let target = parse_target_output(&String::from_utf8_lossy(&output.stdout)).unwrap();
        assert_eq!(
            target.compose_file,
            root.join("docker-compose.yml").to_string_lossy()
        );
        assert_eq!(
            target.image_states.get(&0).map(String::as_str),
            Some("ready")
        );
        assert_eq!(target.containers[0].health, "healthy");
        fs::remove_dir_all(root).unwrap();
    }
}
