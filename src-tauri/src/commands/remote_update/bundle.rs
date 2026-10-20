use super::models::BundlePaths;
use serde::Serialize;
use std::fs::{self};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleLayout {
    pub mode: String,
    pub version: String,
    pub images: Vec<String>,
    pub has_compose: bool,
}

/// 读取本地交付包布局，供部署向导自动判断是镜像模式还是文件模式。
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

    let metadata = if root.join("bundle.json").is_file() {
        let text = fs::read_to_string(root.join("bundle.json"))
            .map_err(|_| "无法读取交付包信息，请检查文件权限".to_string())?;
        Some(
            serde_json::from_str::<serde_json::Value>(&text)
                .map_err(|_| "交付包信息格式有误，请重新选择完整制品".to_string())?,
        )
    } else {
        None
    };
    let declared_mode = metadata.as_ref().and_then(|meta| meta.get("mode"));
    let mode = if let Some(mode) = declared_mode {
        match mode.as_str() {
            Some("file") => "file",
            Some("image") => "image",
            _ => return Err("交付包声明的类型无效，请重新选择完整制品".to_string()),
        }
    } else if root.join("app").is_dir() {
        "file"
    } else if !images.is_empty() {
        "image"
    } else {
        "unknown"
    };

    let version = metadata
        .as_ref()
        .and_then(|meta| meta.get("version"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();

    Ok(BundleLayout {
        mode: mode.to_string(),
        version,
        images,
        has_compose: root.join("deploy/docker-compose.yml").is_file(),
    })
}
pub(super) fn normalize_install_dir(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        crate::commands::profile::deployment_profile()
            .install_dir
            .clone()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn normalize_components(input: Vec<String>) -> Result<Vec<String>, String> {
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

pub(super) fn validate_bundle(
    bundle_dir: &str,
    components: &[String],
) -> Result<BundlePaths, String> {
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
        let artifact = &crate::commands::profile::deployment_profile().backend_artifact;
        let jar = app.join(artifact);
        if jar.is_file()
            && fs::metadata(&jar)
                .map(|meta| meta.len() > 0)
                .unwrap_or(false)
        {
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
pub(super) fn install_bundle_components(bundle_dir: &str) -> Result<Vec<String>, String> {
    let layout = inspect_bundle_layout(bundle_dir.to_string())?;
    match layout.mode.as_str() {
        "image" => Ok(vec!["image".to_string(), "config".to_string()]),
        "file" => {
            let metadata_path = Path::new(bundle_dir).join("bundle.json");
            if metadata_path.is_file() {
                let text = fs::read_to_string(metadata_path)
                    .map_err(|_| "无法读取交付包信息，请检查文件权限".to_string())?;
                let metadata: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|_| "交付包信息格式有误，请重新选择完整制品".to_string())?;
                if let Some(declared) = metadata.get("components") {
                    let declared = declared
                        .as_array()
                        .ok_or_else(|| "交付包的组件清单无效，请重新选择完整制品".to_string())?;
                    let mut components = Vec::new();
                    for component in declared.iter().filter_map(|value| value.as_str()) {
                        if matches!(component, "backend" | "gateway" | "frontend" | "config")
                            && !components.iter().any(|item| item == component)
                        {
                            components.push(component.to_string());
                        }
                    }
                    if !components.iter().any(|item| item == "config") {
                        components.push("config".to_string());
                    }
                    return Ok(components);
                }
            }
            Ok(vec![
                "backend".to_string(),
                "gateway".to_string(),
                "frontend".to_string(),
                "config".to_string(),
            ])
        }
        _ => Err("无法识别交付包内容，请重新选择完整制品".to_string()),
    }
}

pub(in crate::commands) fn validate_install_bundle(bundle_dir: &str) -> Result<(), String> {
    let components = install_bundle_components(bundle_dir)?;
    validate_bundle(bundle_dir, &components).map(|_| ())
}
