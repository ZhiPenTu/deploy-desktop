use std::process::Command;

use serde_json::Value;
use tauri::AppHandle;

use super::deployment_review::{check_local_endpoint, valid_image_reference};
use super::image_library::{export_images_blocking, ImageExportResult};
use super::remote_ssh::{find_binary, resolve_existing_key, run_ssh_script};
use super::remote_update::{compose_service_views, resolve_bundle_file};
use crate::core::process::{self, Input, Options};

#[tauri::command]
pub async fn export_deployment_images(
    window: tauri::Window,
    app: AppHandle,
    bundle_dir: String,
    references: Vec<String>,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<Vec<ImageExportResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = std::fs::canonicalize(bundle_dir)
            .map_err(|_| "无法访问交付包，请重新选择制品".to_string())?;
        let bundle = super::bundle::Bundle::load(&root)?;
        bundle.ensure_compatible()?;
        let compose = resolve_bundle_file(&root, bundle.effective_compose())?;
        let content = std::fs::read_to_string(compose)
            .map_err(|_| "无法读取部署清单，请重新校验交付包".to_string())?;
        let declared: Vec<String> = compose_service_views(&content)
            .map_err(|_| "部署清单格式有误，请检查交付包".to_string())?
            .into_iter()
            .map(|service| service.image)
            .collect();
        validate_selection(&references, &declared)?;
        check_local_endpoint()?;
        let key = resolve_existing_key(key_path, &host)?;
        let output = run_ssh_script(
            &host,
            port,
            &user,
            key.as_deref(),
            "docker info --format '{{.OSType}}/{{.Architecture}}'\n",
        )?;
        if !output.status.success() {
            return Err("无法确认目标主机的 Docker 架构，请检查连接和 Docker 后重试".to_string());
        }
        let target = String::from_utf8_lossy(&output.stdout).trim().to_string();
        for reference in &references {
            let mut command = Command::new(find_binary("docker"));
            command.args(["image", "inspect", "--", reference]);
            let output = process::run(command, Input::Bytes(vec![]), Options::default())?;
            if !output.status.success() {
                return Err(format!("本机缺少镜像 {reference} 或 Docker 不可用，请先在镜像管理中准备该镜像，或选择离线镜像包"));
            }
            let images: Vec<Value> = serde_json::from_slice(&output.stdout)
                .map_err(|_| "无法确认本机镜像信息，请检查 Docker 后重试".to_string())?;
            let image = images.first()
                .ok_or_else(|| "未读取到本机镜像信息，请重新准备镜像".to_string())?;
            validate_platform(image, &target, reference)?;
        }
        export_images_blocking(&window, &app, references)
    })
    .await
    .map_err(|_| "导出镜像任务未完成，请检查本机 Docker 后重试".to_string())?
}

fn validate_selection(references: &[String], declared: &[String]) -> Result<(), String> {
    if references.is_empty()
        || references
            .iter()
            .any(|reference| !valid_image_reference(reference) || !declared.contains(reference))
    {
        return Err("所选镜像不在部署清单中，请重新比对后重试".to_string());
    }
    Ok(())
}

fn architecture(value: &str) -> &str {
    match value {
        "x86_64" | "amd64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        other => other,
    }
}

fn validate_platform(image: &Value, target: &str, reference: &str) -> Result<(), String> {
    let (target_os, target_arch) = target
        .split_once('/')
        .filter(|(os, arch)| !os.is_empty() && !arch.is_empty())
        .ok_or_else(|| "无法确认目标主机架构，请重新检测环境".to_string())?;
    let image_os = image.get("Os").and_then(Value::as_str).unwrap_or("");
    let image_arch = image
        .get("Architecture")
        .and_then(Value::as_str)
        .unwrap_or("");
    if image_os != target_os || architecture(image_arch) != architecture(target_arch) {
        return Err(format!("镜像 {reference} 的架构 {image_os}/{image_arch} 与目标主机 {target} 不一致，请先准备对应架构的镜像"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transfer_accepts_only_complete_declared_references() {
        let reference = format!("demo/api@sha256:{}", "a".repeat(64));
        let declared = vec![reference.clone()];
        assert!(validate_selection(&[reference], &declared).is_ok());
        assert!(validate_selection(&["demo/api:latest".into()], &declared).is_err());
        assert!(validate_selection(&["--output".into()], &declared).is_err());
        assert!(validate_selection(&[], &declared).is_err());
    }

    #[test]
    fn transfer_rejects_wrong_or_unknown_platform() {
        let image = json!({"Os": "linux", "Architecture": "arm64"});
        assert!(validate_platform(&image, "linux/aarch64", "demo/api").is_ok());
        assert!(validate_platform(&image, "linux/x86_64", "demo/api").is_err());
        assert!(validate_platform(&image, "windows/arm64", "demo/api").is_err());
        assert!(validate_platform(&image, "", "demo/api").is_err());
        assert!(validate_platform(&json!({}), "linux/arm64", "demo/api").is_err());
        assert!(validate_platform(
            &json!({"Os": "linux", "Architecture": "amd64"}),
            "linux/x86_64",
            "demo/api"
        )
        .is_ok());
    }
}
