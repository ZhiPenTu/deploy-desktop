use super::plan::{image_present, image_tags_for_target};
use serde::Serialize;
use std::fs::{self};
use std::path::{Path, PathBuf};

const CONFIG_PREVIEW_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeServiceView {
    pub name: String,
    pub image: String,
    pub container_name: String,
    pub restart: String,
    pub ports: Vec<String>,
    pub depends_on: Vec<String>,
    pub profiles: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeImageView {
    pub image_ref: String,
    pub match_ref: String,
    pub services: Vec<String>,
    pub local_file: String,
    pub local_bytes: u64,
    /// 镜像包来源：images.json / profile / filename / none。
    pub match_source: String,
    pub docker_present: bool,
    /// ready（包与镜像都在）/ to_import（有包未导入）/ loaded（仅本机已有）/
    /// missing（包与镜像都缺失）/ unconfirmed（Docker 状态无法确认）。
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleConfigView {
    pub file: String,
    pub kind: String,
    pub size_bytes: u64,
    pub content: String,
    pub truncated: bool,
    pub services: Vec<ComposeServiceView>,
    pub images: Vec<ComposeImageView>,
    pub image_dir: String,
    pub image_dir_exists: bool,
    pub manifest: String,
    pub image_check: String,
    pub issues: Vec<String>,
}

/// 读取交付包内的配置文件原文，并把 compose 的 `services.*.image` 解析成镜像清单，
/// 与本地镜像包和 Docker 已加载镜像逐一对比。
///
/// 配置文件是唯一事实来源：界面展示的镜像清单直接来自 compose，service 归属也一并返回，
/// 不再依赖手工维护的 images 列表，避免清单与实际编排漂移。
#[tauri::command]
pub fn inspect_bundle_config(
    bundle_dir: String,
    file: String,
    image_dir: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<BundleConfigView, String> {
    let root = PathBuf::from(bundle_dir.trim());
    if !root.is_dir() {
        return Err(format!("交付包目录不存在: {}", root.display()));
    }
    let root = fs::canonicalize(&root)
        .map_err(|error| format!("无法访问交付包目录 {}: {error}", root.display()))?;

    let target = resolve_bundle_file(&root, &file)?;
    let relative = target
        .strip_prefix(&root)
        .unwrap_or(&target)
        .to_string_lossy()
        .replace('\\', "/");

    let bytes = fs::read(&target).map_err(|error| format!("读取配置文件失败：{error}"))?;
    let size_bytes = bytes.len() as u64;
    let truncated = bytes.len() > CONFIG_PREVIEW_LIMIT;
    let content =
        String::from_utf8_lossy(&bytes[..bytes.len().min(CONFIG_PREVIEW_LIMIT)]).to_string();

    let mut issues: Vec<String> = Vec::new();
    let lower = target
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let is_yaml = lower.ends_with(".yml") || lower.ends_with(".yaml");

    let mut services: Vec<ComposeServiceView> = Vec::new();
    let mut kind = "text".to_string();
    if is_yaml {
        match compose_service_views(&content) {
            Ok(list) => {
                kind = if list.is_empty() {
                    "yaml".to_string()
                } else {
                    "compose".to_string()
                };
                services = list;
            }
            Err(error) => issues.push(error),
        }
    }
    if truncated {
        issues.push("配置文件超过 1 MB，界面只展示前 1 MB 内容".to_string());
    }

    // 与镜像包管理保持一致：给了目录时优先用其下的 images/，否则用目录本身；
    // 未给目录时先看交付包内的 images/，没有就退回交付包根目录。
    let image_dir_path = match image_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        Some(dir) => {
            let nested = dir.join("images");
            if nested.is_dir() {
                nested
            } else {
                dir
            }
        }
        None => {
            let nested = root.join("images");
            if nested.is_dir() {
                nested
            } else {
                root.clone()
            }
        }
    };
    let image_dir_exists = image_dir_path.is_dir();
    if !image_dir_exists {
        issues.push(format!(
            "本地镜像包目录不存在：{}",
            image_dir_path.display()
        ));
    }

    let manifest = crate::commands::image_manager::load_image_manifest(&image_dir_path);
    let profile = crate::commands::profile::deployment_profile();
    let files = scan_image_files(&image_dir_path);

    let manifest_label = if manifest.iter().any(|entry| !entry.images.is_empty()) {
        "images.json"
    } else if !profile.images.is_empty() {
        "profile"
    } else {
        "none"
    };

    let (docker_tags, image_check) =
        image_tags_for_target(host.as_deref(), port, user.as_deref(), key_path.as_deref());
    let normalized_tags: Vec<String> = docker_tags
        .iter()
        .map(|tag| normalize_image_ref(tag))
        .collect();

    let mut images: Vec<ComposeImageView> = Vec::new();
    for service in &services {
        let reference = service.image.trim();
        if reference.is_empty() {
            continue;
        }
        let match_ref = normalize_image_ref(reference);
        if let Some(existing) = images.iter_mut().find(|item| item.match_ref == match_ref) {
            existing.services.push(service.name.clone());
            continue;
        }
        let (local_file, match_source) =
            match_tarball(&match_ref, &files, &manifest, &profile.images);
        let docker_present = image_present(&normalized_tags, &match_ref);
        let status = compose_image_status(local_file.is_some(), docker_present, &image_check);
        images.push(ComposeImageView {
            image_ref: reference.to_string(),
            match_ref,
            services: vec![service.name.clone()],
            local_file: local_file
                .as_ref()
                .map(|(name, _)| name.clone())
                .unwrap_or_default(),
            local_bytes: local_file.map(|(_, size)| size).unwrap_or(0),
            match_source: match_source.to_string(),
            docker_present,
            status: status.to_string(),
        });
    }

    Ok(BundleConfigView {
        file: relative,
        kind,
        size_bytes,
        content,
        truncated,
        services,
        images,
        image_dir: image_dir_path.to_string_lossy().to_string(),
        image_dir_exists,
        manifest: manifest_label.to_string(),
        image_check,
        issues,
    })
}

pub(in crate::commands) fn resolve_bundle_file(root: &Path, file: &str) -> Result<PathBuf, String> {
    let relative = file.trim();
    if relative.is_empty() {
        return Err("未指定要读取的配置文件".to_string());
    }
    let candidate = Path::new(relative);
    if candidate.is_absolute()
        || candidate
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(format!("配置文件路径不合法：{relative}"));
    }
    let target = root.join(candidate);
    let canonical = fs::canonicalize(&target)
        .map_err(|error| format!("无法读取配置文件 {relative}：{error}"))?;
    if !canonical.starts_with(root) {
        return Err(format!("配置文件不在交付包目录内：{relative}"));
    }
    if !canonical.is_file() {
        return Err(format!("配置文件不存在：{relative}"));
    }
    Ok(canonical)
}

pub(in crate::commands) fn compose_service_views(
    content: &str,
) -> Result<Vec<ComposeServiceView>, String> {
    let document: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("YAML 解析失败：{error}"))?;
    let Some(services) = document
        .get("services")
        .and_then(|value| value.as_mapping())
    else {
        return Ok(Vec::new());
    };

    let mut views = Vec::new();
    for (key, body) in services {
        let Some(name) = key.as_str() else {
            continue;
        };
        views.push(ComposeServiceView {
            name: name.to_string(),
            image: yaml_text(body.get("image")),
            container_name: yaml_text(body.get("container_name")),
            restart: yaml_text(body.get("restart")),
            ports: yaml_list(body.get("ports")),
            depends_on: compose_depends_on(body.get("depends_on")),
            profiles: yaml_list(body.get("profiles")),
        });
    }
    Ok(views)
}

fn yaml_text(value: Option<&serde_yaml::Value>) -> String {
    match value {
        Some(serde_yaml::Value::String(text)) => text.trim().to_string(),
        Some(serde_yaml::Value::Number(number)) => number.to_string(),
        Some(serde_yaml::Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

fn yaml_list(value: Option<&serde_yaml::Value>) -> Vec<String> {
    let Some(serde_yaml::Value::Sequence(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| yaml_text(Some(item)))
        .filter(|text| !text.is_empty())
        .collect()
}

fn compose_depends_on(value: Option<&serde_yaml::Value>) -> Vec<String> {
    match value {
        Some(serde_yaml::Value::Mapping(map)) => map
            .keys()
            .filter_map(|key| key.as_str().map(|name| name.to_string()))
            .collect(),
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .map(|item| yaml_text(Some(item)))
            .filter(|text| !text.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

pub(in crate::commands) fn normalize_image_ref(reference: &str) -> String {
    let value = reference.trim();
    let without_digest = value.split('@').next().unwrap_or(value).trim();
    let without_registry = without_digest
        .strip_prefix("docker.io/")
        .or_else(|| without_digest.strip_prefix("index.docker.io/"))
        .unwrap_or(without_digest);
    without_registry
        .strip_prefix("library/")
        .unwrap_or(without_registry)
        .to_string()
}

pub(super) fn compose_image_status(
    has_package: bool,
    docker_present: bool,
    image_check: &str,
) -> &'static str {
    match (has_package, docker_present) {
        (true, true) => "ready",
        (true, false) => "to_import",
        (false, true) => "loaded",
        (false, false) if image_check == "unavailable" => "unconfirmed",
        (false, false) => "missing",
    }
}

pub(in crate::commands) fn scan_image_files(dir: &Path) -> Vec<(String, u64)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_lowercase();
        if !(lower.ends_with(".tar") || lower.ends_with(".tgz") || lower.ends_with(".tar.gz")) {
            continue;
        }
        let bytes = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        files.push((name, bytes));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

/// 镜像包匹配顺序：交付包清单 images.json → 部署配置里的文件名映射 → 文件名推断。
/// 推断只做兜底，界面会标注来源，避免把猜测当成清单结论。
pub(in crate::commands) fn match_tarball(
    match_ref: &str,
    files: &[(String, u64)],
    manifest: &[crate::commands::image_manager::ImageManifestEntry],
    profile_images: &[crate::commands::profile::ImageProfile],
) -> (Option<(String, u64)>, &'static str) {
    let find_file = |name: &str| files.iter().find(|(file, _)| file == name).cloned();

    for entry in manifest {
        if entry
            .images
            .iter()
            .any(|image| normalize_image_ref(image) == match_ref)
        {
            if let Some(hit) = find_file(&entry.file) {
                return (Some(hit), "images.json");
            }
        }
    }

    for image in profile_images {
        if normalize_image_ref(&image.image_ref) == match_ref {
            if let Some(hit) = find_file(&image.file) {
                return (Some(hit), "profile");
            }
        }
    }

    if let Some(hit) = heuristic_tarball(match_ref, files) {
        return (Some(hit), "filename");
    }

    (None, "none")
}

fn heuristic_tarball(match_ref: &str, files: &[(String, u64)]) -> Option<(String, u64)> {
    let repo = match_ref.split(':').next().unwrap_or(match_ref);
    let last = repo.rsplit('/').next().unwrap_or(repo).to_lowercase();
    let tokens: Vec<&str> = last
        .split(|ch| ch == '-' || ch == '_' || ch == '.')
        .filter(|token| token.len() >= 4)
        .collect();
    if tokens.is_empty() {
        return None;
    }

    for (name, bytes) in files {
        let stem = name
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".tgz")
            .trim_end_matches(".tar")
            .to_lowercase();
        if stem.len() < 4 {
            continue;
        }
        if tokens
            .iter()
            .any(|token| *token == stem.as_str() || token.contains(&stem))
        {
            return Some((name.clone(), *bytes));
        }
    }
    None
}
