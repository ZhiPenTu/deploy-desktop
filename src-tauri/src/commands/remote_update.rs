use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use tauri::Emitter;

use super::docker_ops::ContainerStatusItem;
use super::remote_ssh::{
    explain_ssh_failure, find_binary, resolve_existing_key, run_ssh_script, shell_quote,
    ssh_base_command,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteArtifactInfo {
    pub exists: bool,
    pub size_bytes: u64,
    pub modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDeploymentStatus {
    pub deployed: bool,
    pub install_dir: String,
    pub deploy_dir: String,
    pub compose_file: String,
    pub docker_available: bool,
    pub docker_running: bool,
    pub compose_available: bool,
    pub server_jar: RemoteArtifactInfo,
    pub gateway_classes: RemoteArtifactInfo,
    pub gateway_lib: RemoteArtifactInfo,
    pub frontend_dist: RemoteArtifactInfo,
    pub containers: Vec<ContainerStatusItem>,
    pub running_containers: usize,
    pub total_containers: usize,
    pub issues: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteUpdateProgress {
    pub step: String,
    pub percent: usize,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteUpdateResult {
    pub success: bool,
    pub components: Vec<String>,
    pub backup_dir: Option<String>,
    pub output: String,
    pub duration_secs: u64,
    pub status: Option<RemoteDeploymentStatus>,
}

struct BundlePaths {
    server_jar: Option<PathBuf>,
    gateway_classes: Option<PathBuf>,
    gateway_lib: Option<PathBuf>,
    frontend_dist: Option<PathBuf>,
    compose_file: Option<PathBuf>,
    compose_lan_file: Option<PathBuf>,
    nginx_dir: Option<PathBuf>,
    oceanbase_dir: Option<PathBuf>,
    image_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleLayout {
    pub mode: String,
    pub version: String,
    pub images: Vec<String>,
    pub has_compose: bool,
}

/// 读取本地交付包布局，供服务更新页自动判断是镜像模式还是文件模式。
#[tauri::command]
pub fn inspect_bundle_layout(bundle_dir: String) -> Result<BundleLayout, String> {
    let root = PathBuf::from(bundle_dir.trim());
    if !root.is_dir() {
        return Err(format!("交付包目录不存在: {}", root.display()));
    }

    let mut images = Vec::new();
    let images_dir = root.join("images");
    if let Ok(entries) = fs::read_dir(&images_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !path.is_file() {
                continue;
            }
            if name.ends_with(".tar") || name.ends_with(".tgz") || name.ends_with(".tar.gz") {
                images.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    images.sort();

    let mode = if !images.is_empty() {
        "image"
    } else if root.join("app").is_dir() {
        "file"
    } else {
        "unknown"
    };

    let mut version = String::new();
    if let Ok(text) = fs::read_to_string(root.join("bundle.json")) {
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&text) {
            version = meta
                .get("version")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
        }
    }

    Ok(BundleLayout {
        mode: mode.to_string(),
        version,
        images,
        has_compose: root.join("deploy/docker-compose.yml").is_file(),
    })
}

#[tauri::command]
pub fn inspect_remote_deployment(
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
pub fn get_remote_containers_status(
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
pub fn get_remote_service_logs(
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

#[tauri::command]
pub async fn update_remote_deployment(
    window: tauri::Window,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
    install_dir: String,
    components: Vec<String>,
    backup: bool,
    dry_run: bool,
) -> Result<RemoteUpdateResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        update_remote_deployment_blocking(
            &window,
            host,
            port,
            user,
            key_path,
            bundle_dir,
            install_dir,
            components,
            backup,
            dry_run,
        )
    })
    .await
    .map_err(|e| format!("更新任务执行失败: {e}"))?
}

fn update_remote_deployment_blocking(
    window: &tauri::Window,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
    install_dir: String,
    components: Vec<String>,
    backup: bool,
    dry_run: bool,
) -> Result<RemoteUpdateResult, String> {
    let started = Instant::now();
    let components = normalize_components(components)?;
    if components.is_empty() {
        return Err("请至少选择一个更新组件".to_string());
    }

    emit_progress(window, "校验更新包", 3, "running", "检查本地更新包内容...");
    let bundle = validate_bundle(&bundle_dir, &components)?;

    let key = resolve_existing_key(key_path, &host)?;
    let install_dir = normalize_install_dir(&install_dir);
    let deploy_dir = format!("{install_dir}/deploy");

    emit_progress(window, "检查目标机", 8, "running", "确认目标机部署目录与容器状态...");
    let status = inspect_remote_deployment_inner(&host, port, &user, key.as_deref(), &install_dir)?;
    if !status.deployed {
        return Err(format!(
            "目标机未检测到已部署服务（{}）。请先使用部署向导完成首次安装。",
            status.deploy_dir
        ));
    }
    if !status.docker_available {
        return Err("目标机未安装 Docker，无法执行服务更新。".to_string());
    }
    if !status.docker_running {
        return Err("目标机 Docker 守护进程未运行，请先启动 Docker。".to_string());
    }
    if !status.compose_available {
        return Err("目标机缺少 docker compose，无法执行服务更新。".to_string());
    }
    ensure_required_containers(&components, &status)?;

    if backup {
        let free_kb = remote_free_kb(&host, port, &user, key.as_deref(), &install_dir)?;
        if free_kb < 2 * 1024 * 1024 {
            return Err(format!(
                "目标机部署分区剩余空间仅 {:.1} GB，更新备份至少需要 2 GB。请清理空间或关闭“更新前备份”。",
                free_kb as f64 / 1024.0 / 1024.0
            ));
        }
    }

    if dry_run {
        emit_progress(
            window,
            "预检通过",
            100,
            "success",
            "更新包与目标机状态检查通过，未修改目标机",
        );
        return Ok(RemoteUpdateResult {
            success: true,
            components,
            backup_dir: None,
            output: format!(
                "预检通过：目标机 {} 已部署，更新包与所需容器检查通过。未上传文件，也未重启服务。",
                install_dir
            ),
            duration_secs: started.elapsed().as_secs(),
            status: Some(status),
        });
    }

    let mut output_log = String::new();
    let backup_dir = if backup {
        emit_progress(window, "创建备份", 15, "running", "备份现有 jar、网关文件与前端产物...");
        let dir = create_remote_backup(&host, port, &user, key.as_deref(), &install_dir, &deploy_dir)?;
        output_log.push_str(&format!("备份目录: {dir}\n"));
        emit_progress(window, "创建备份", 20, "success", &format!("已备份到 {dir}"));
        Some(dir)
    } else {
        emit_progress(window, "创建备份", 20, "skipped", "已关闭更新前备份");
        None
    };

    if let Some(path) = &bundle.server_jar {
        emit_progress(window, "上传后端服务", 28, "running", "上传后端 fat jar...");
        let remote = format!(
            "{install_dir}/backend-java/server/target/{}",
            super::profile::deployment_profile().backend_artifact
        );
        upload_file_replace(&host, port, &user, key.as_deref(), path, &remote)?;
        output_log.push_str(&format!("已上传: {remote}\n"));
        emit_progress(window, "上传后端服务", 38, "success", "后端 fat jar 已上传");
    }

    if let Some(path) = &bundle.gateway_classes {
        emit_progress(window, "上传网关 classes", 42, "running", "上传网关 classes 目录...");
        let remote = format!("{install_dir}/backend-java/gateway-app/target/classes");
        upload_dir_replace(&host, port, &user, key.as_deref(), path, &remote)?;
        output_log.push_str(&format!("已替换: {remote}\n"));
        emit_progress(window, "上传网关 classes", 48, "success", "网关 classes 已替换");
    }

    if let Some(path) = &bundle.gateway_lib {
        emit_progress(window, "上传网关依赖", 50, "running", "上传网关 lib 目录...");
        let remote = format!("{install_dir}/backend-java/gateway-app/target/lib");
        upload_dir_replace(&host, port, &user, key.as_deref(), path, &remote)?;
        output_log.push_str(&format!("已替换: {remote}\n"));
        emit_progress(window, "上传网关依赖", 55, "success", "网关 lib 已替换");
    }

    if let Some(path) = &bundle.frontend_dist {
        emit_progress(window, "上传前端页面", 58, "running", "上传前端 dist 目录...");
        let remote = format!("{deploy_dir}/frontend_dist");
        upload_dir_replace(&host, port, &user, key.as_deref(), path, &remote)?;
        output_log.push_str(&format!("已替换: {remote}\n"));
        emit_progress(window, "上传前端页面", 64, "success", "前端 dist 已替换");
    }

    if let Some(path) = &bundle.compose_file {
        emit_progress(window, "上传部署配置", 66, "running", "上传 compose 与挂载配置...");
        upload_file_replace(
            &host,
            port,
            &user,
            key.as_deref(),
            path,
            &format!("{deploy_dir}/docker-compose.yml"),
        )?;
        if let Some(lan) = &bundle.compose_lan_file {
            upload_file_replace(
                &host,
                port,
                &user,
                key.as_deref(),
                lan,
                &format!("{deploy_dir}/docker-compose.lan.yml"),
            )?;
        }
        if let Some(nginx) = &bundle.nginx_dir {
            upload_dir_replace(
                &host,
                port,
                &user,
                key.as_deref(),
                nginx,
                &format!("{deploy_dir}/nginx"),
            )?;
        }
        if let Some(oceanbase) = &bundle.oceanbase_dir {
            upload_dir_replace(
                &host,
                port,
                &user,
                key.as_deref(),
                oceanbase,
                &format!("{deploy_dir}/oceanbase"),
            )?;
        }
        output_log.push_str(&format!("已更新部署配置: {deploy_dir}\n"));
        emit_progress(window, "上传部署配置", 72, "success", "部署配置已更新");
    }

    if !bundle.image_files.is_empty() {
        emit_progress(
            window,
            "上传镜像包",
            74,
            "running",
            "上传镜像包并导入 Docker 镜像，大文件需要一些时间...",
        );
        let names: Vec<String> = bundle
            .image_files
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|value| value.to_string_lossy().to_string())
            })
            .collect();
        let loaded = super::image_manager::push_images_blocking(
            window,
            &host,
            port,
            &user,
            key.clone(),
            &bundle_dir,
            &install_dir,
            names,
            true,
        )?;
        for line in loaded {
            output_log.push_str(&format!("{line}\n"));
        }
        emit_progress(window, "导入镜像", 76, "success", "镜像包已导入目标机");
    }

    emit_progress(window, "应用更新", 78, "running", "重启所选服务并等待健康检查...");
    let apply_output = apply_remote_update(
        &host,
        port,
        &user,
        key.as_deref(),
        &deploy_dir,
        &components,
    )?;
    output_log.push_str(&apply_output);
    emit_progress(window, "应用更新", 94, "success", "所选服务已重启并通过健康检查");

    let final_status =
        inspect_remote_deployment_inner(&host, port, &user, key.as_deref(), &install_dir).ok();
    emit_progress(window, "更新完成", 100, "success", "服务更新完成");

    Ok(RemoteUpdateResult {
        success: true,
        components,
        backup_dir,
        output: output_log,
        duration_secs: started.elapsed().as_secs(),
        status: final_status,
    })
}

fn normalize_install_dir(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        super::profile::deployment_profile().install_dir.clone()
    } else {
        trimmed.to_string()
    }
}

fn normalize_components(input: Vec<String>) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for item in input {
        let key = item.trim().to_lowercase();
        match key.as_str() {
            "backend" | "gateway" | "frontend" | "config" | "image" => {
                if !result.contains(&key) {
                    result.push(key);
                }
            }
            other => return Err(format!("不支持的更新组件: {other}")),
        }
    }
    Ok(result)
}


fn is_image_bundle(root: &Path) -> bool {
    root.join("images").is_dir() && !root.join("app").is_dir()
}

fn validate_bundle(bundle_dir: &str, components: &[String]) -> Result<BundlePaths, String> {
    let root = PathBuf::from(bundle_dir.trim());
    if !root.is_dir() {
        return Err(format!("更新包目录不存在: {}", root.display()));
    }

    let app = root.join("app");
    let deploy = root.join("deploy");
    let mut paths = BundlePaths {
        server_jar: None,
        gateway_classes: None,
        gateway_lib: None,
        frontend_dist: None,
        compose_file: None,
        compose_lan_file: None,
        nginx_dir: None,
        oceanbase_dir: None,
        image_files: Vec::new(),
    };
    let mut missing = Vec::new();

    if components.iter().any(|c| c == "backend") {
        let artifact = &super::profile::deployment_profile().backend_artifact;
        let jar = app.join(artifact);
        if jar.is_file() && fs::metadata(&jar).map(|meta| meta.len() > 0).unwrap_or(false) {
            paths.server_jar = Some(jar);
        } else {
            missing.push(format!("app/{artifact}"));
        }
    }

    if components.iter().any(|c| c == "gateway") {
        let classes = app.join("gateway-classes");
        if classes.is_dir() {
            paths.gateway_classes = Some(classes);
        } else {
            missing.push("app/gateway-classes/".to_string());
        }
        let lib = app.join("gateway-lib");
        if lib.is_dir() {
            paths.gateway_lib = Some(lib);
        }
    }

    if components.iter().any(|c| c == "frontend") {
        let dist = app.join("frontend-dist");
        if dist.join("index.html").is_file() {
            paths.frontend_dist = Some(dist);
        } else {
            missing.push("app/frontend-dist/index.html".to_string());
        }
    }

    if components.iter().any(|c| c == "config") {
        let compose = deploy.join("docker-compose.yml");
        if compose.is_file() {
            paths.compose_file = Some(compose);
        } else {
            missing.push("deploy/docker-compose.yml".to_string());
        }
        let compose_lan = deploy.join("docker-compose.lan.yml");
        if compose_lan.is_file() {
            paths.compose_lan_file = Some(compose_lan);
        }
        if deploy.join("nginx").is_dir() {
            paths.nginx_dir = Some(deploy.join("nginx"));
        }
        if deploy.join("oceanbase").is_dir() {
            paths.oceanbase_dir = Some(deploy.join("oceanbase"));
        }
    }

    if components.iter().any(|c| c == "image") {
        let images = root.join("images");
        if images.is_dir() {
            let mut found = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&images) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path
                        .file_name()
                        .map(|value| value.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    if path.is_file()
                        && (name.ends_with(".tar")
                            || name.ends_with(".tgz")
                            || name.ends_with(".tar.gz"))
                    {
                        found.push(path);
                    }
                }
            }
            if found.is_empty() {
                missing.push("images/*.tar.gz".to_string());
            } else {
                found.sort();
                paths.image_files = found;
            }
        } else {
            missing.push("images/".to_string());
        }
    }

    if is_image_bundle(&root)
        && components
            .iter()
            .any(|c| matches!(c.as_str(), "backend" | "gateway" | "frontend"))
    {
        return Err(
            "该交付包是镜像模式（只有 images/ 与 deploy/，没有 app/ 产物）。请只勾选「镜像」和「部署配置」，镜像会通过 docker load 导入。"
                .to_string(),
        );
    }

    if !missing.is_empty() {
        return Err(format!(
            "更新包缺少所选组件所需的文件：{}。请检查更新目录是否包含完整的 app/ 与 deploy/ 内容。",
            missing.join("、")
        ));
    }

    Ok(paths)
}

fn ensure_required_containers(
    components: &[String],
    status: &RemoteDeploymentStatus,
) -> Result<(), String> {
    let profile = super::profile::deployment_profile();
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

fn inspect_remote_deployment_inner(
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
        artifact = super::profile::deployment_profile().backend_artifact
    );

    for (container, _) in super::profile::deployment_profile().container_entries() {
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
                let service_key = super::profile::deployment_profile()
                    .service_key_for_container(&name);
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

    let deploy_exists = flags.get("DEPLOY_EXISTS").map(|v| v == "yes").unwrap_or(false);
    let compose_file = flags.get("COMPOSE_FILE").cloned().unwrap_or_default();
    let docker_available = flags.get("DOCKER").map(|v| v == "yes").unwrap_or(false);
    let docker_running = flags.get("DOCKER_RUNNING").map(|v| v == "yes").unwrap_or(false);
    let compose_available = flags.get("COMPOSE").map(|v| v == "yes").unwrap_or(false);
    let server_jar = artifact_from_flags(&flags, "SERVER_JAR");
    let gateway_classes = artifact_from_flags(&flags, "GATEWAY_CLASSES");
    let gateway_lib = artifact_from_flags(&flags, "GATEWAY_LIB");
    let frontend_dist = artifact_from_flags(&flags, "FRONTEND_DIST");

    if containers.is_empty() {
        for (name, service_key) in super::profile::deployment_profile().container_entries() {
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

fn remote_free_kb(
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

fn create_remote_backup(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install_dir: &str,
    deploy_dir: &str,
) -> Result<String, String> {
    let script = format!(
        r#"set -e
INSTALL={install}
DEPLOY={deploy}
STAMP=$(date +%Y%m%d-%H%M%S)
BACKUP="$INSTALL/backup/update-$STAMP"
mkdir -p "$BACKUP"
if [ -f "$INSTALL/backend-java/server/target/{artifact}" ]; then
  mkdir -p "$BACKUP/server"
  cp -a "$INSTALL/backend-java/server/target/{artifact}" "$BACKUP/server/"
fi
if [ -d "$INSTALL/backend-java/gateway-app/target/classes" ]; then
  mkdir -p "$BACKUP/gateway"
  cp -a "$INSTALL/backend-java/gateway-app/target/classes" "$BACKUP/gateway/"
fi
if [ -d "$INSTALL/backend-java/gateway-app/target/lib" ]; then
  mkdir -p "$BACKUP/gateway"
  cp -a "$INSTALL/backend-java/gateway-app/target/lib" "$BACKUP/gateway/"
fi
if [ -d "$DEPLOY/frontend_dist" ]; then
  mkdir -p "$BACKUP/frontend"
  cp -a "$DEPLOY/frontend_dist" "$BACKUP/frontend/"
fi
if [ -f "$DEPLOY/docker-compose.yml" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/docker-compose.yml" "$BACKUP/deploy/"
fi
if [ -d "$DEPLOY/nginx" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/nginx" "$BACKUP/deploy/"
fi
if [ -d "$DEPLOY/oceanbase" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/oceanbase" "$BACKUP/deploy/"
fi
echo "BACKUP_DIR=$BACKUP"
"#,
        install = shell_quote(install_dir),
        deploy = shell_quote(deploy_dir),
        artifact = super::profile::deployment_profile().backend_artifact
    );
    let output = run_ssh_script(host, port, user, key, &script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "创建远程备份失败: {}",
            explain_ssh_failure(&stderr, key)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("BACKUP_DIR=").map(|value| value.trim().to_string()))
        .ok_or_else(|| "备份完成但未返回备份目录".to_string())
}

fn upload_file(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote: &str,
) -> Result<(), String> {
    let parent = Path::new(remote)
        .parent()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());
    let mkdir_script = format!("mkdir -p {}", shell_quote(&parent));
    let mkdir_output = run_ssh_script(host, port, user, key, &mkdir_script)?;
    if !mkdir_output.status.success() {
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key));
    }

    let mut child = ssh_base_command(host, port, user, key)
        .arg(format!("cat > {}", shell_quote(remote)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动上传命令失败: {e}"))?;

    let mut file =
        File::open(local).map_err(|e| format!("打开本地文件 {} 失败: {e}", local.display()))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "无法打开 SSH 标准输入".to_string())?;
        io::copy(&mut file, stdin).map_err(|e| format!("写入远程文件失败: {e}"))?;
    }
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待上传完成失败: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!(
            "上传 {} 到 {} 失败: {}",
            local.display(),
            remote,
            explain_ssh_failure(&stderr, key)
        ))
    }
}

fn upload_file_replace(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote: &str,
) -> Result<(), String> {
    let temp = format!("{remote}.tt-new");
    upload_file(host, port, user, key, local, &temp)?;
    let swap = format!(
        "mv -f {temp} {dest}",
        temp = shell_quote(&temp),
        dest = shell_quote(remote)
    );
    let output = run_ssh_script(host, port, user, key, &swap)?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let _ = run_ssh_script(host, port, user, key, &format!("rm -f {}", shell_quote(&temp)));
        Err(format!(
            "替换 {} 失败: {}",
            remote,
            explain_ssh_failure(&stderr, key)
        ))
    }
}

fn upload_dir_replace(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote: &str,
) -> Result<(), String> {
    let temp = format!("{remote}.tt-new");
    let prep = format!(
        "rm -rf {temp} && mkdir -p {temp}",
        temp = shell_quote(&temp)
    );
    let prep_output = run_ssh_script(host, port, user, key, &prep)?;
    if !prep_output.status.success() {
        let stderr = String::from_utf8_lossy(&prep_output.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key));
    }

    let mut tar = Command::new(find_binary("tar"))
        .args(["-czf", "-", "-C"])
        .arg(local)
        .arg(".")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动本地 tar 失败: {e}"))?;
    let mut ssh = ssh_base_command(host, port, user, key)
        .arg(format!("tar -xzf - -C {}", shell_quote(&temp)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动远程解包失败: {e}"))?;

    let mut tar_stdout = tar
        .stdout
        .take()
        .ok_or_else(|| "无法读取 tar 输出".to_string())?;
    let mut ssh_stdin = ssh
        .stdin
        .take()
        .ok_or_else(|| "无法打开 SSH 标准输入".to_string())?;
    io::copy(&mut tar_stdout, &mut ssh_stdin).map_err(|e| format!("传输目录失败: {e}"))?;
    drop(ssh_stdin);

    let tar_status = tar.wait().map_err(|e| format!("等待 tar 结束失败: {e}"))?;
    let ssh_output = ssh
        .wait_with_output()
        .map_err(|e| format!("等待远程解包结束失败: {e}"))?;

    if !tar_status.success() {
        let stderr = String::from_utf8_lossy(&ssh_output.stderr).trim().to_string();
        let _ = run_ssh_script(host, port, user, key, &format!("rm -rf {}", shell_quote(&temp)));
        return Err(format!(
            "打包本地目录 {} 失败{}",
            local.display(),
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        ));
    }
    if !ssh_output.status.success() {
        let stderr = String::from_utf8_lossy(&ssh_output.stderr).trim().to_string();
        let _ = run_ssh_script(host, port, user, key, &format!("rm -rf {}", shell_quote(&temp)));
        return Err(format!(
            "远程解包失败: {}",
            explain_ssh_failure(&stderr, key)
        ));
    }

    let swap = format!(
        "rm -rf {dest} && mv {temp} {dest}",
        dest = shell_quote(remote),
        temp = shell_quote(&temp)
    );
    let swap_output = run_ssh_script(host, port, user, key, &swap)?;
    if swap_output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&swap_output.stderr).trim().to_string();
        Err(format!(
            "替换远程目录 {} 失败: {}",
            remote,
            explain_ssh_failure(&stderr, key)
        ))
    }
}

fn apply_remote_update(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    deploy_dir: &str,
    components: &[String],
) -> Result<String, String> {
    let has_backend = components.iter().any(|component| component == "backend");
    let has_gateway = components.iter().any(|component| component == "gateway");
    let has_frontend = components.iter().any(|component| component == "frontend");
    let has_config = components.iter().any(|component| component == "config");
    let has_image = components.iter().any(|component| component == "image");

    let profile = super::profile::deployment_profile();
    let mut restart_services: Vec<String> = Vec::new();
    if has_backend {
        restart_services.extend(profile.component_services("backend"));
    }
    if has_gateway {
        restart_services.extend(profile.component_services("gateway"));
    }
    if has_frontend {
        restart_services.extend(profile.component_services("frontend"));
    }
    if has_image {
        restart_services.extend(profile.component_services("backend"));
        restart_services.extend(profile.component_services("gateway"));
        restart_services.extend(profile.component_services("frontend"));
    }
    restart_services.sort();
    restart_services.dedup();

    let wait_services: Vec<String> = if has_config || has_image {
        profile
            .containers
            .iter()
            .map(|container| container.service_key.clone())
            .collect()
    } else {
        restart_services.clone()
    };

    let mut script = format!(
        r#"set -euo pipefail
DEPLOY={deploy}
cd "$DEPLOY"
if [ -f docker-compose.lan.yml ]; then
  COMPOSE_FILE=docker-compose.lan.yml
elif [ -f docker-compose.yml ]; then
  COMPOSE_FILE=docker-compose.yml
else
  echo "部署目录缺少 docker-compose.yml" >&2
  exit 4
fi
compose() {{
  if docker compose version >/dev/null 2>&1; then
    docker compose -f "$COMPOSE_FILE" "$@"
  else
    docker-compose -f "$COMPOSE_FILE" "$@"
  fi
}}
"#,
        deploy = shell_quote(deploy_dir)
    );

    if has_config {
        script.push_str("echo \"==> 应用部署配置 (docker compose up -d)\"\ncompose up -d\n");
    }

    if has_frontend {
        let frontend_service = profile
            .component_services("frontend")
            .into_iter()
            .next()
            .ok_or_else(|| "profile 未配置前端服务".to_string())?;
        let frontend_container = profile.container_name(&frontend_service);
        script.push_str(&format!(
            r#"echo "==> 更新前端静态页面"
# frontend_dist 以只读 bind mount 提供给官方 nginx 镜像，静态文件按请求读取，
# 不再 docker cp 进容器；容器未运行时用 compose 拉起。
if [ "$(docker inspect --format '{{{{.State.Status}}}}' {frontend} 2>/dev/null || echo missing)" != "running" ]; then
  compose up -d frontend
fi
"#,
            frontend = shell_quote(&frontend_container)
        ));
    }

    if !restart_services.is_empty() {
        let apply_line = if has_image {
            format!(
                "echo \"==> 按新镜像重建服务: {}\"\ncompose up -d {}\n",
                restart_services.join(" "),
                restart_services.join(" ")
            )
        } else {
            format!(
                "echo \"==> 重启服务: {}\"\ncompose restart {}\n",
                restart_services.join(" "),
                restart_services.join(" ")
            )
        };
        script.push_str(&apply_line);
    }

    if !wait_services.is_empty() {
        script.push_str("echo \"==> 等待服务健康检查\"\n");
        for service in &wait_services {
            let container = super::profile::deployment_profile().container_name(service);
            script.push_str(&format!(
                r#"ready=no
for i in $(seq 1 45); do
  health=$(docker inspect --format '{{{{if .State.Health}}}}{{{{.State.Health.Status}}}}{{{{else}}}}none{{{{end}}}}' {container} 2>/dev/null || echo missing)
  state=$(docker inspect --format '{{{{.State.Status}}}}' {container} 2>/dev/null || echo missing)
  if [ "$health" = "healthy" ] || {{ [ "$health" = "none" ] && [ "$state" = "running" ]; }}; then
    ready=yes
    break
  fi
  sleep 2
done
if [ "$ready" != "yes" ]; then
  echo "服务 {container} 未就绪 (health=$health, state=$state)" >&2
  docker logs --tail 40 {container} 2>&1 || true
  exit 4
fi
"#,
                container = shell_quote(&container)
            ));
        }
    }

    script.push_str("echo \"==> 当前容器状态\"\ncompose ps\n");

    let output = run_ssh_script(host, port, user, key, &script)?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{stdout}\n{stderr}").trim().to_string();
    if output.status.success() {
        Ok(combined)
    } else {
        Err(format!(
            "应用更新失败: {}\n{}",
            explain_ssh_failure(&stderr, key),
            combined
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallBundleResult {
    pub success: bool,
    pub install_dir: String,
    pub deploy_dir: String,
    pub uploaded: Vec<String>,
    pub output: String,
}

#[tauri::command]
pub async fn install_bundle(
    window: tauri::Window,
    mode: String,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
    install_dir: String,
) -> Result<InstallBundleResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        install_bundle_blocking(
            &window,
            &mode,
            &host,
            port,
            &user,
            key_path.as_deref(),
            &bundle_dir,
            &install_dir,
        )
    })
    .await
    .map_err(|e| format!("分发任务执行失败: {e}"))?
}

fn install_bundle_blocking(
    window: &tauri::Window,
    mode: &str,
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<&str>,
    bundle_dir: &str,
    install_dir: &str,
) -> Result<InstallBundleResult, String> {
    let mode = mode.trim();
    if mode != "local" && mode != "remote" {
        return Err(format!("不支持的分发模式: {mode}"));
    }
    let components = vec![
        "backend".to_string(),
        "gateway".to_string(),
        "frontend".to_string(),
        "config".to_string(),
    ];
    emit_progress(window, "校验交付包", 4, "running", "检查交付包结构...");
    let bundle = validate_bundle(bundle_dir, &components)?;
    let install_dir = normalize_install_dir(install_dir);
    let deploy_dir = format!("{install_dir}/deploy");
    let artifact = super::profile::deployment_profile().backend_artifact.clone();

    let mut uploaded: Vec<String> = Vec::new();
    if mode == "local" {
        emit_progress(window, "分发交付包", 20, "running", "复制交付包到部署目录...");
        uploaded = install_bundle_local(&bundle, Path::new(bundle_dir), &install_dir)?;
    } else {
        let key = resolve_existing_key(key_path.map(str::to_string), host)?;

        if let Some(path) = &bundle.server_jar {
            emit_progress(window, "分发后端服务", 20, "running", "上传后端 fat jar...");
            let remote = format!("{install_dir}/backend-java/server/target/{artifact}");
            upload_file_replace(host, port, user, key.as_deref(), path, &remote)?;
            uploaded.push(remote);
        }
        if let Some(path) = &bundle.gateway_classes {
            emit_progress(window, "分发网关 classes", 35, "running", "上传网关 classes...");
            let remote = format!("{install_dir}/backend-java/gateway-app/target/classes");
            upload_dir_replace(host, port, user, key.as_deref(), path, &remote)?;
            uploaded.push(remote);
        }
        if let Some(path) = &bundle.gateway_lib {
            emit_progress(window, "分发网关依赖", 50, "running", "上传网关依赖 jar...");
            let remote = format!("{install_dir}/backend-java/gateway-app/target/lib");
            upload_dir_replace(host, port, user, key.as_deref(), path, &remote)?;
            uploaded.push(remote);
        }
        if let Some(path) = &bundle.frontend_dist {
            emit_progress(window, "分发前端页面", 65, "running", "上传前端静态资源...");
            let remote = format!("{deploy_dir}/frontend_dist");
            upload_dir_replace(host, port, user, key.as_deref(), path, &remote)?;
            uploaded.push(remote);
        }
        if let Some(path) = &bundle.compose_file {
            emit_progress(window, "分发部署配置", 80, "running", "上传 compose 与挂载配置...");
            let remote = format!("{deploy_dir}/docker-compose.yml");
            upload_file_replace(host, port, user, key.as_deref(), path, &remote)?;
            uploaded.push(remote);
        }
        if let Some(path) = &bundle.compose_lan_file {
            let remote = format!("{deploy_dir}/docker-compose.lan.yml");
            upload_file_replace(host, port, user, key.as_deref(), path, &remote)?;
            uploaded.push(remote);
        }
        if let Some(path) = &bundle.nginx_dir {
            let remote = format!("{deploy_dir}/nginx");
            upload_dir_replace(host, port, user, key.as_deref(), path, &remote)?;
            uploaded.push(remote);
        }
        if let Some(path) = &bundle.oceanbase_dir {
            let remote = format!("{deploy_dir}/oceanbase");
            upload_dir_replace(host, port, user, key.as_deref(), path, &remote)?;
            uploaded.push(remote);
        }
    }

    emit_progress(window, "分发完成", 100, "success", "交付包已分发到部署目录");
    Ok(InstallBundleResult {
        success: true,
        install_dir: install_dir.clone(),
        deploy_dir: deploy_dir.clone(),
        uploaded: uploaded.clone(),
        output: format!(
            "交付包已分发到 {install_dir}，共写入 {} 个位置。下一步可启动容器栈。",
            uploaded.len()
        ),
    })
}

fn install_bundle_local(
    bundle: &BundlePaths,
    bundle_root: &Path,
    install_dir: &str,
) -> Result<Vec<String>, String> {
    let artifact = super::profile::deployment_profile().backend_artifact.clone();
    let server_dir = Path::new(install_dir)
        .join("backend-java")
        .join("server")
        .join("target");
    let gateway_dir = Path::new(install_dir)
        .join("backend-java")
        .join("gateway-app")
        .join("target");
    let deploy_dir = Path::new(install_dir).join("deploy");
    let mut written = Vec::new();

    if let Some(path) = &bundle.server_jar {
        let dest = server_dir.join(&artifact);
        copy_file_replace(path, &dest)?;
        written.push(dest.to_string_lossy().to_string());
    }
    if let Some(path) = &bundle.gateway_classes {
        let dest = gateway_dir.join("classes");
        copy_dir_replace(path, &dest)?;
        written.push(dest.to_string_lossy().to_string());
    }
    if let Some(path) = &bundle.gateway_lib {
        let dest = gateway_dir.join("lib");
        copy_dir_replace(path, &dest)?;
        written.push(dest.to_string_lossy().to_string());
    }
    if let Some(path) = &bundle.frontend_dist {
        let dest = deploy_dir.join("frontend_dist");
        copy_dir_replace(path, &dest)?;
        written.push(dest.to_string_lossy().to_string());
    }

    let source_deploy = bundle_root.join("deploy");
    if source_deploy.is_dir() {
        copy_dir_merge(&source_deploy, &deploy_dir)?;
        written.push(deploy_dir.to_string_lossy().to_string());
    }
    Ok(written)
}

fn copy_file_replace(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    fs::copy(src, dest).map_err(|e| {
        format!(
            "复制 {} 到 {} 失败: {e}",
            src.display(),
            dest.display()
        )
    })?;
    Ok(())
}

fn copy_dir_replace(src: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        fs::remove_dir_all(dest)
            .map_err(|e| format!("清理目录 {} 失败: {e}", dest.display()))?;
    }
    copy_dir_merge(src, dest)
}

fn copy_dir_merge(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("创建目录 {} 失败: {e}", dest.display()))?;
    let entries = fs::read_dir(src).map_err(|e| format!("读取目录 {} 失败: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("读取文件类型失败: {e}"))?;
        let target = dest.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_merge(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target).map_err(|e| {
                format!(
                    "复制 {} 到 {} 失败: {e}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn emit_progress(
    window: &tauri::Window,
    step: &str,
    percent: usize,
    status: &str,
    message: &str,
) {
    let _ = window.emit(
        "remote-update-progress",
        RemoteUpdateProgress {
            step: step.to_string(),
            percent,
            status: status.to_string(),
            message: message.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn local_install_places_bundle_into_compose_mount_layout() {
        let root = std::env::temp_dir().join(format!(
            "deploy-desktop-install-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let bundle = root.join("bundle");
        let install = root.join("install");
        write(&bundle.join("app/server.jar"), "jar");
        write(&bundle.join("app/frontend-dist/index.html"), "<html></html>");
        write(&bundle.join("app/gateway-classes/Application.class"), "x");
        write(&bundle.join("app/gateway-lib/dependency.jar"), "x");
        write(&bundle.join("deploy/docker-compose.yml"), "name: demo");
        write(&bundle.join("deploy/docker-compose.lan.yml"), "name: lan");
        write(&bundle.join("deploy/nginx/udp-lb.conf"), "events {}");
        write(&bundle.join("deploy/nginx/frontend-default.conf"), "server {}");
        write(&bundle.join("deploy/nginx/frontend-upstream.conf"), "upstream backend {}");
        write(&bundle.join("deploy/nginx/frontend-main.conf"), "http {}");

        let paths = BundlePaths {
            server_jar: Some(bundle.join("app/server.jar")),
            gateway_classes: Some(bundle.join("app/gateway-classes")),
            gateway_lib: Some(bundle.join("app/gateway-lib")),
            frontend_dist: Some(bundle.join("app/frontend-dist")),
            compose_file: Some(bundle.join("deploy/docker-compose.yml")),
            compose_lan_file: Some(bundle.join("deploy/docker-compose.lan.yml")),
            nginx_dir: Some(bundle.join("deploy/nginx")),
            oceanbase_dir: None,
            image_files: Vec::new(),
        };

        install_bundle_local(&paths, &bundle, install.to_str().unwrap()).unwrap();

        let artifact = &super::super::profile::deployment_profile().backend_artifact;
        assert!(install
            .join("backend-java/server/target")
            .join(artifact)
            .is_file());
        assert!(install
            .join("backend-java/gateway-app/target/classes/Application.class")
            .is_file());
        assert!(install
            .join("backend-java/gateway-app/target/lib/dependency.jar")
            .is_file());
        assert!(install.join("deploy/frontend_dist/index.html").is_file());
        assert!(install.join("deploy/docker-compose.yml").is_file());
        assert!(install.join("deploy/docker-compose.lan.yml").is_file());
        assert!(install.join("deploy/nginx/udp-lb.conf").is_file());
        assert!(install.join("deploy/nginx/frontend-default.conf").is_file());
        assert!(install.join("deploy/nginx/frontend-upstream.conf").is_file());
        assert!(install.join("deploy/nginx/frontend-main.conf").is_file());

        fs::remove_dir_all(&root).ok();
    }
}
