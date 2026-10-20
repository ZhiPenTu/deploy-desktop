use super::bundle::{install_bundle_components, normalize_install_dir, validate_bundle};
use super::models::BundlePaths;
use super::progress::emit_progress;
use super::transfer::{upload_dir_replace, upload_file_replace};
use crate::commands::remote_ssh::resolve_existing_key;
use serde::{Deserialize, Serialize};
use std::fs::{self};
use std::path::Path;

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
    let components = install_bundle_components(bundle_dir)?;
    emit_progress(window, "校验交付包", 4, "running", "检查交付包结构...");
    let bundle = validate_bundle(bundle_dir, &components)?;
    let install_dir = normalize_install_dir(install_dir);
    let deploy_dir = format!("{install_dir}/deploy");
    let artifact = crate::commands::profile::deployment_profile()
        .backend_artifact
        .clone();

    let mut uploaded: Vec<String> = Vec::new();
    if mode == "local" {
        emit_progress(
            window,
            "分发交付包",
            20,
            "running",
            "复制交付包到部署目录...",
        );
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
            emit_progress(
                window,
                "分发网关 classes",
                35,
                "running",
                "上传网关 classes...",
            );
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
            emit_progress(
                window,
                "分发部署配置",
                80,
                "running",
                "上传 compose 与挂载配置...",
            );
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
        if !bundle.image_files.is_empty() {
            emit_progress(
                window,
                "分发镜像包",
                90,
                "running",
                "上传镜像包并在目标机导入，大文件需要一些时间...",
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
                host,
                port,
                user,
                key.clone(),
                bundle_dir,
                &install_dir,
                names,
                true,
            )?;
            uploaded.extend(loaded);
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
pub(super) fn install_bundle_local(
    bundle: &BundlePaths,
    bundle_root: &Path,
    install_dir: &str,
) -> Result<Vec<String>, String> {
    let artifact = crate::commands::profile::deployment_profile()
        .backend_artifact
        .clone();
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

    // 镜像模式交付包：镜像包必须一起落到部署目录，后续「加载镜像」与
    // 「启动容器栈」才能在本机找到 images/*.tar.gz。
    let source_images = bundle_root.join("images");
    let target_images = Path::new(install_dir).join("images");
    if source_images.is_dir() && source_images != target_images {
        copy_dir_merge(&source_images, &target_images)?;
        written.push(target_images.to_string_lossy().to_string());
    }
    Ok(written)
}

fn copy_file_replace(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    fs::copy(src, dest)
        .map_err(|e| format!("复制 {} 到 {} 失败: {e}", src.display(), dest.display()))?;
    Ok(())
}

fn copy_dir_replace(src: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|e| format!("清理目录 {} 失败: {e}", dest.display()))?;
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
