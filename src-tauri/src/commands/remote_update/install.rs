use super::bundle::{install_components, normalize_install_dir};
use super::progress::emit_progress;
use super::transfer::{upload_dir_replace, upload_file_replace};
use crate::commands::bundle::{write_deployed_record, ArtifactKind, Bundle};
use crate::commands::projects;
use crate::commands::remote_ssh::resolve_existing_key;
use serde::{Deserialize, Serialize};
use std::fs::{self};
use std::path::{Path, PathBuf};

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
    emit_progress(window, "校验交付包", 4, "running", "检查交付包结构与文件摘要...");
    let bundle = Bundle::load(Path::new(bundle_dir.trim()))?;
    bundle.ensure_compatible()?;
    bundle.verify_payload()?;

    let install_dir = normalize_install_dir(install_dir);
    if install_dir.is_empty() {
        return Err("请先为当前项目填写部署目录".to_string());
    }
    let install_root = PathBuf::from(&install_dir);
    let deploy_root = install_root.join(bundle.deploy_source_rel());
    let components = install_components(&bundle);
    let selected = bundle.select_components(&components)?;
    let resolved = bundle.resolve_artifacts(&selected, &install_root, &deploy_root)?;
    if resolved.is_empty() {
        return Err("交付包未声明可分发的产物，请重新构建制品".to_string());
    }

    let mut uploaded: Vec<String> = Vec::new();
    if mode == "local" {
        emit_progress(
            window,
            "分发交付包",
            20,
            "running",
            "复制交付包到部署目录...",
        );
        uploaded = install_bundle_local(&resolved, bundle_dir, &install_root)?;
        copy_bundle_images(&bundle, &install_root, &mut uploaded)?;
    } else {
        let key = resolve_existing_key(key_path.map(str::to_string), host)?;
        for (index, artifact) in resolved.iter().enumerate() {
            let percent = 20 + index * 55 / resolved.len().max(1);
            emit_progress(
                window,
                "分发产物",
                percent,
                "running",
                &format!("上传 {}", artifact.label),
            );
            let remote = artifact.target.to_string_lossy().to_string();
            match artifact.kind {
                ArtifactKind::File => upload_file_replace(
                    host,
                    port,
                    user,
                    key.as_deref(),
                    &artifact.source,
                    &remote,
                )?,
                ArtifactKind::Dir => upload_dir_replace(
                    host,
                    port,
                    user,
                    key.as_deref(),
                    &artifact.source,
                    &remote,
                )?,
            }
            uploaded.push(remote);
        }
        if bundle.has_bundle_images() {
            emit_progress(
                window,
                "分发镜像包",
                90,
                "running",
                "上传镜像包并在目标机导入，大文件需要一些时间...",
            );
            let names: Vec<String> = bundle
                .image_files()
                .iter()
                .filter_map(|path| {
                    path.file_name()
                        .map(|value| value.to_string_lossy().to_string())
                })
                .collect();
            if !names.is_empty() {
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
    }

    let project_id = projects::active_project()
        .map(|project| project.id)
        .unwrap_or_default();
    let record = bundle.deployed_record(&project_id);
    if let Err(error) = write_deployed_record(&install_root, &record) {
        log::warn!("写入部署记录失败: {error}");
    }

    emit_progress(window, "分发完成", 100, "success", "交付包已分发到部署目录");
    Ok(InstallBundleResult {
        success: true,
        install_dir: install_dir.clone(),
        deploy_dir: deploy_root.to_string_lossy().to_string(),
        uploaded: uploaded.clone(),
        output: format!(
            "交付包已分发到 {install_dir}，共写入 {} 个位置。下一步可启动容器栈。",
            uploaded.len()
        ),
    })
}

fn install_bundle_local(
    artifacts: &[crate::commands::bundle::ResolvedArtifact],
    _bundle_dir: &str,
    _install_root: &Path,
) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for artifact in artifacts {
        match artifact.kind {
            ArtifactKind::File => copy_file_replace(&artifact.source, &artifact.target)?,
            ArtifactKind::Dir => copy_dir_replace(&artifact.source, &artifact.target)?,
        }
        written.push(artifact.target.to_string_lossy().to_string());
    }
    Ok(written)
}

fn copy_bundle_images(
    bundle: &Bundle,
    install_root: &Path,
    written: &mut Vec<String>,
) -> Result<(), String> {
    let files = bundle.image_files();
    if files.is_empty() {
        return Ok(());
    }
    let target_dir = install_root.join("images");
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("创建目录 {} 失败: {e}", target_dir.display()))?;
    for source in files {
        let name = source
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .ok_or_else(|| "镜像包文件名无效".to_string())?;
        let target = target_dir.join(name);
        copy_file_replace(&source, &target)?;
        written.push(target.to_string_lossy().to_string());
    }
    Ok(())
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
