use super::bundle::{load_bundle, normalize_install_dir};
use super::inspect::{inspect_remote_deployment_inner, remote_free_kb};
use super::models::RemoteUpdateResult;
use super::progress::emit_progress;
use super::transfer::{create_remote_backup, upload_dir_replace, upload_file_replace};
use crate::commands::bundle::{ArtifactKind, ArtifactRoot, Bundle};
use crate::commands::projects;
use crate::commands::remote_ssh::{
    explain_ssh_failure, resolve_existing_key, run_ssh_script, shell_quote,
};
use std::path::Path;
use std::time::Instant;

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

#[allow(clippy::too_many_arguments)]
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
    emit_progress(window, "校验更新包", 3, "running", "检查本地更新包内容...");
    let (bundle, components) = load_bundle(&bundle_dir, &components)?;
    bundle.verify_payload()?;

    let key = resolve_existing_key(key_path, &host)?;
    let install_dir = normalize_install_dir(&install_dir);

    emit_progress(
        window,
        "检查目标机",
        8,
        "running",
        "确认目标机部署目录与容器状态...",
    );
    let status =
        inspect_remote_deployment_inner(&host, port, &user, key.as_deref(), &install_dir)?;
    let deploy_dir = status.deploy_dir.clone();
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
    let selected = bundle.select_components(&components)?;
    let resolved = bundle.resolve_artifacts(
        &selected,
        Path::new(&install_dir),
        Path::new(&deploy_dir),
    )?;
    ensure_required_services(&bundle, &components, &status)?;

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
        emit_progress(
            window,
            "创建备份",
            15,
            "running",
            "备份将被替换的产物与部署配置...",
        );
        let dir = create_remote_backup(
            &host,
            port,
            &user,
            key.as_deref(),
            &install_dir,
            &deploy_dir,
            &resolved,
        )?;
        output_log.push_str(&format!("备份目录: {dir}\n"));
        emit_progress(
            window,
            "创建备份",
            20,
            "success",
            &format!("已备份到 {dir}"),
        );
        Some(dir)
    } else {
        emit_progress(window, "创建备份", 20, "skipped", "已关闭更新前备份");
        None
    };

    for (index, artifact) in resolved.iter().enumerate() {
        let percent = 28 + index * 45 / resolved.len().max(1);
        emit_progress(
            window,
            "上传产物",
            percent,
            "running",
            &format!("上传 {}", artifact.label),
        );
        let remote = artifact.target.to_string_lossy().to_string();
        match artifact.kind {
            ArtifactKind::File => upload_file_replace(
                &host,
                port,
                &user,
                key.as_deref(),
                &artifact.source,
                &remote,
            )?,
            ArtifactKind::Dir => upload_dir_replace(
                &host,
                port,
                &user,
                key.as_deref(),
                &artifact.source,
                &remote,
            )?,
        }
        output_log.push_str(&format!("已写入 {remote}\n"));
    }

    if components.iter().any(|item| item == "image") {
        emit_progress(
            window,
            "导入镜像",
            78,
            "running",
            "上传镜像包并在目标机导入...",
        );
        let names: Vec<String> = bundle
            .image_files()
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|value| value.to_string_lossy().to_string())
            })
            .collect();
        if names.is_empty() {
            return Err("该交付包未包含离线镜像，请取消镜像更新或重新下载制品".to_string());
        }
        let loaded = crate::commands::image_manager::push_images_blocking(
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
        output_log.push_str(&format!("已导入 {} 个镜像包\n", loaded.len()));
    }

    emit_progress(
        window,
        "应用更新",
        88,
        "running",
        "重启受影响的服务并等待就绪",
    );
    let output = apply_remote_update(
        &host,
        port,
        &user,
        key.as_deref(),
        &deploy_dir,
        &components,
        &bundle,
    )?;
    output_log.push_str(&output);

    let project_id = projects::active_project()
        .map(|project| project.id)
        .unwrap_or_default();
    let record = bundle.deployed_record(&project_id);
    if let Ok(text) = serde_json::to_string(&record) {
        let script = format!(
            "mkdir -p {}/.deploy-desktop && printf '%s' {} > {}/.deploy-desktop/deployed.json",
            shell_quote(&install_dir),
            shell_quote(&text),
            shell_quote(&install_dir)
        );
        if let Err(error) = run_ssh_script(&host, port, &user, key.as_deref(), &script) {
            log::warn!("写入目标机部署记录失败: {error}");
        }
    }

    emit_progress(
        window,
        "更新完成",
        100,
        "success",
        "所选组件已更新并通过就绪检查",
    );
    let status =
        inspect_remote_deployment_inner(&host, port, &user, key.as_deref(), &install_dir).ok();
    Ok(RemoteUpdateResult {
        success: true,
        components,
        backup_dir,
        output: output_log,
        duration_secs: started.elapsed().as_secs(),
        status,
    })
}

fn selected_services(bundle: &Bundle, components: &[String]) -> Vec<String> {
    let mut services: Vec<String> = Vec::new();
    for key in components {
        if key == "image" {
            continue;
        }
        if let Some(component) = bundle.component(key) {
            for service in &component.services {
                if !services.contains(service) {
                    services.push(service.clone());
                }
            }
        }
    }
    services.sort();
    services
}

/// 目标机上实际使用的 Compose 文件（相对部署目录）。
fn compose_file_on_target(bundle: &Bundle) -> String {
    let source = bundle.effective_compose();
    match bundle.artifact_for_source(source) {
        Some(artifact) if artifact.root == ArtifactRoot::Deploy => artifact.path.trim().to_string(),
        _ => Path::new(source)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| source.to_string()),
    }
}

fn is_config_selected(bundle: &Bundle, components: &[String]) -> bool {
    let compose_sources = [
        bundle.descriptor.runtime.compose.trim().to_string(),
        bundle.effective_compose().to_string(),
    ];
    components.iter().any(|key| {
        if key == "image" {
            return false;
        }
        bundle
            .component(key)
            .map(|component| {
                component.artifacts.iter().any(|artifact| {
                    bundle
                        .artifact(artifact)
                        .map(|spec| compose_sources.iter().any(|source| spec.source == *source))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    })
}

fn ensure_required_services(
    bundle: &Bundle,
    components: &[String],
    status: &super::models::RemoteDeploymentStatus,
) -> Result<(), String> {
    let required = selected_services(bundle, components);
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

fn apply_remote_update(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    deploy_dir: &str,
    components: &[String],
    bundle: &Bundle,
) -> Result<String, String> {
    let restart_services = selected_services(bundle, components);
    let config_selected = is_config_selected(bundle, components) || components.iter().any(|c| c == "image");
    let wait_services: Vec<String> = if config_selected {
        bundle
            .descriptor
            .runtime
            .services
            .iter()
            .map(|service| service.key.clone())
            .collect()
    } else {
        restart_services.clone()
    };

    let compose_file = compose_file_on_target(bundle);
    let mut script = format!(
        r#"set -euo pipefail
DEPLOY={deploy}
cd "$DEPLOY"
COMPOSE_FILE={compose_file}
if [ ! -f "$COMPOSE_FILE" ]; then
  echo "部署目录缺少 {compose_file}，请先完成首次安装" >&2
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
        deploy = shell_quote(deploy_dir),
        compose_file = shell_quote(&compose_file)
    );

    if config_selected {
        script.push_str(
            "echo \"==> 应用部署配置 (docker compose up -d)\"\ncompose up -d --pull never --no-build --remove-orphans\n",
        );
    }

    if !restart_services.is_empty() {
        let action = if components.iter().any(|item| item == "image") {
            "up -d --pull never --no-build --remove-orphans"
        } else {
            "restart"
        };
        script.push_str(&format!(
            "echo \"==> 应用服务变更: {}\"\ncompose {} {}\n",
            restart_services.join(" "),
            action,
            restart_services.join(" ")
        ));
    }

    if !wait_services.is_empty() {
        script.push_str("echo \"==> 等待服务健康检查\"\n");
        for service in &wait_services {
            let container = bundle.container_name(service);
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

