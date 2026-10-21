use super::bundle::{normalize_components, normalize_install_dir, validate_bundle};
use super::inspect::{ensure_required_containers, inspect_remote_deployment_inner, remote_free_kb};
use super::models::RemoteUpdateResult;
use super::progress::emit_progress;
use super::transfer::{create_remote_backup, upload_dir_replace, upload_file_replace};
use crate::commands::remote_ssh::{
    explain_ssh_failure, resolve_existing_key, run_ssh_script, shell_quote,
};
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

    emit_progress(
        window,
        "检查目标机",
        8,
        "running",
        "确认目标机部署目录与容器状态...",
    );
    let status = inspect_remote_deployment_inner(&host, port, &user, key.as_deref(), &install_dir)?;
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
        emit_progress(
            window,
            "创建备份",
            15,
            "running",
            "备份现有 jar、网关文件与前端产物...",
        );
        let dir = create_remote_backup(
            &host,
            port,
            &user,
            key.as_deref(),
            &install_dir,
            &deploy_dir,
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

    if let Some(path) = &bundle.server_jar {
        emit_progress(window, "上传后端服务", 28, "running", "上传后端 fat jar...");
        let remote = format!(
            "{install_dir}/backend-java/server/target/{}",
            crate::commands::profile::deployment_profile().backend_artifact
        );
        upload_file_replace(&host, port, &user, key.as_deref(), path, &remote)?;
        output_log.push_str(&format!("已上传: {remote}\n"));
        emit_progress(window, "上传后端服务", 38, "success", "后端 fat jar 已上传");
    }

    if let Some(path) = &bundle.gateway_classes {
        emit_progress(
            window,
            "上传网关 classes",
            42,
            "running",
            "上传网关 classes 目录...",
        );
        let remote = format!("{install_dir}/backend-java/gateway-app/target/classes");
        upload_dir_replace(&host, port, &user, key.as_deref(), path, &remote)?;
        output_log.push_str(&format!("已替换: {remote}\n"));
        emit_progress(
            window,
            "上传网关 classes",
            48,
            "success",
            "网关 classes 已替换",
        );
    }

    if let Some(path) = &bundle.gateway_lib {
        emit_progress(
            window,
            "上传网关依赖",
            50,
            "running",
            "上传网关 lib 目录...",
        );
        let remote = format!("{install_dir}/backend-java/gateway-app/target/lib");
        upload_dir_replace(&host, port, &user, key.as_deref(), path, &remote)?;
        output_log.push_str(&format!("已替换: {remote}\n"));
        emit_progress(window, "上传网关依赖", 55, "success", "网关 lib 已替换");
    }

    if let Some(path) = &bundle.frontend_dist {
        emit_progress(
            window,
            "上传前端页面",
            58,
            "running",
            "上传前端 dist 目录...",
        );
        let remote = format!("{deploy_dir}/frontend_dist");
        upload_dir_replace(&host, port, &user, key.as_deref(), path, &remote)?;
        output_log.push_str(&format!("已替换: {remote}\n"));
        emit_progress(window, "上传前端页面", 64, "success", "前端 dist 已替换");
    }

    if let Some(path) = &bundle.compose_file {
        emit_progress(
            window,
            "上传部署配置",
            66,
            "running",
            "上传 compose 与挂载配置...",
        );
        upload_file_replace(
            &host,
            port,
            &user,
            key.as_deref(),
            path,
            &format!("{deploy_dir}/docker-compose.yml"),
        )?;
        let lan = bundle.compose_lan_file.as_ref().or_else(|| {
            (status.compose_file == "docker-compose.lan.yml").then_some(path)
        });
        if let Some(lan) = lan {
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
        for line in loaded {
            output_log.push_str(&format!("{line}\n"));
        }
        emit_progress(window, "导入镜像", 76, "success", "镜像包已导入目标机");
    }

    emit_progress(
        window,
        "应用更新",
        78,
        "running",
        "重启所选服务并等待健康检查...",
    );
    let apply_output =
        apply_remote_update(&host, port, &user, key.as_deref(), &deploy_dir, &components)?;
    output_log.push_str(&apply_output);
    emit_progress(
        window,
        "应用更新",
        94,
        "success",
        "所选服务已重启并通过健康检查",
    );

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

    let profile = crate::commands::profile::deployment_profile();
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
            let container = crate::commands::profile::deployment_profile().container_name(service);
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
