use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use super::image_library::{login_docker, TempDockerConfig};
use super::registry_sources::{source_access, source_credentials, RegistrySourceAccess};

use super::remote_ssh::{
    find_binary, resolve_existing_key, run_ssh_script, shell_quote,
};

const MANIFEST_API_VERSION: &str = "deploy.desktop/v1";
const MANIFEST_MAX_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageManifest {
    pub api_version: String,
    pub kind: String,
    pub metadata: ManifestMetadata,
    pub spec: ManifestSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestMetadata {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub min_tool_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestSpec {
    pub registry: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    pub images: Vec<ManifestImage>,
    #[serde(default)]
    pub artifacts: Vec<ManifestArtifact>,
    #[serde(default)]
    pub checksums: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestImage {
    pub name: String,
    pub repository: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub platform: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestArtifact {
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ResolvedImage {
    pub name: String,
    pub repository: String,
    pub reference: String,
    pub digest: String,
    pub platform: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ManifestSummary {
    pub source: String,
    pub manifest: ImageManifest,
    pub images: Vec<ResolvedImage>,
    pub total_size_bytes: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ImagePullResult {
    pub name: String,
    pub reference: String,
    pub digest: String,
    pub platform: Option<String>,
    pub exported_path: Option<String>,
    pub size_bytes: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RegistryProgress {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub image: String,
    pub percent: usize,
    pub message: String,
    pub finished: bool,
}

#[tauri::command]
pub async fn load_image_manifest(
    app: AppHandle,
    source: String,
    registry_source_id: Option<String>,
) -> Result<ManifestSummary, String> {
    crate::commands::blocking::run(move || load_image_manifest_blocking(app, source, registry_source_id))
        .await
}

pub(crate) fn load_image_manifest_blocking(
    app: AppHandle,
    source: String,
    registry_source_id: Option<String>,
) -> Result<ManifestSummary, String> {
    let access = source_access(&app, registry_source_id.as_deref())?;
    resolve_with_source(load_and_resolve(&source)?, access.as_ref())
}

#[tauri::command]
pub async fn pull_manifest_images_remote(
    window: tauri::Window,
    app: AppHandle,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    source: String,
    username: Option<String>,
    password: Option<String>,
    image_names: Vec<String>,
    export_dir: Option<String>,
    registry_source_id: Option<String>,
) -> Result<Vec<ImagePullResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let access = source_access(&app, registry_source_id.as_deref())?;
        let summary = resolve_with_source(load_and_resolve(&source)?, access.as_ref())?;
        let (username, password) = match access {
            Some(access) => source_credentials(access, password)?,
            None => (username, password),
        };
        pull_remote_blocking(
            &window,
            &host,
            port,
            &user,
            key_path,
            summary,
            username,
            password,
            image_names,
            export_dir,
        )
    })
    .await
    .map_err(|e| format!("拉取镜像任务失败: {e}"))?
}

#[tauri::command]
pub async fn pull_manifest_images_local(
    window: tauri::Window,
    app: AppHandle,
    source: String,
    username: Option<String>,
    password: Option<String>,
    image_names: Vec<String>,
    export_dir: Option<String>,
    registry_source_id: Option<String>,
) -> Result<Vec<ImagePullResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let access = source_access(&app, registry_source_id.as_deref())?;
        let summary = resolve_with_source(load_and_resolve(&source)?, access.as_ref())?;
        let (username, password) = match access {
            Some(access) => source_credentials(access, password)?,
            None => (username, password),
        };
        pull_local_blocking(
            &window,
            summary,
            username,
            password,
            image_names,
            export_dir,
        )
    })
    .await
    .map_err(|e| format!("拉取镜像任务失败: {e}"))?
}

fn resolve_with_source(mut summary: ManifestSummary, access: Option<&RegistrySourceAccess>) -> Result<ManifestSummary, String> {
    if let Some(access) = access {
        summary.manifest.spec.registry = access.source.address.clone();
        if !access.source.namespace.is_empty() {
            summary.manifest.spec.namespace = Some(access.source.namespace.clone());
        }
        validate_manifest(&summary.manifest)?;
        summary.images = resolve_images(&summary.manifest)?;
    }
    Ok(summary)
}

fn load_and_resolve(source: &str) -> Result<ManifestSummary, String> {
    let (content, normalized_source) = read_manifest_source(source)?;
    let manifest: ImageManifest = serde_yaml::from_str(&content)
        .map_err(|e| format!("清单解析失败，请检查 YAML/JSON 格式: {e}"))?;
    validate_manifest(&manifest)?;
    let images = resolve_images(&manifest)?;
    let total_size_bytes = images.iter().map(|image| image.size_bytes).sum();
    let warnings = manifest_warnings(&manifest, &images);

    Ok(ManifestSummary {
        source: normalized_source,
        manifest,
        images,
        total_size_bytes,
        warnings,
    })
}

fn read_manifest_source(source: &str) -> Result<(String, String), String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("请填写清单地址或本地文件路径".to_string());
    }

    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("deploy-desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("创建清单下载客户端失败: {e}"))?;
        let response = client
            .get(trimmed)
            .send()
            .map_err(|e| format!("下载清单失败: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("下载清单失败: HTTP {}", response.status()));
        }
        if let Some(length) = response.content_length() {
            if length > MANIFEST_MAX_BYTES {
                return Err("清单文件超过 5 MB 限制".to_string());
            }
        }
        let bytes = response
            .bytes()
            .map_err(|e| format!("读取清单响应失败: {e}"))?;
        if bytes.len() as u64 > MANIFEST_MAX_BYTES {
            return Err("清单文件超过 5 MB 限制".to_string());
        }
        let content = String::from_utf8(bytes.to_vec())
            .map_err(|_| "清单不是有效的 UTF-8 文本".to_string())?;
        return Ok((content, trimmed.to_string()));
    }

    let path = expand_home(trimmed);
    let metadata = fs::metadata(&path)
        .map_err(|e| format!("读取本地清单失败: {} ({e})", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("清单路径不是文件: {}", path.display()));
    }
    if metadata.len() > MANIFEST_MAX_BYTES {
        return Err("清单文件超过 5 MB 限制".to_string());
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("读取本地清单失败: {} ({e})", path.display()))?;
    Ok((content, path.display().to_string()))
}

fn expand_home(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn validate_manifest(manifest: &ImageManifest) -> Result<(), String> {
    if manifest.api_version != MANIFEST_API_VERSION {
        return Err(format!(
            "不支持的清单版本: {}，当前仅支持 {}",
            manifest.api_version, MANIFEST_API_VERSION
        ));
    }
    if manifest.kind != "ImageBundle" {
        return Err(format!(
            "清单类型必须是 ImageBundle，当前为 {}",
            manifest.kind
        ));
    }
    if manifest.metadata.name.trim().is_empty() {
        return Err("清单 metadata.name 不能为空".to_string());
    }
    if manifest.metadata.version.trim().is_empty() {
        return Err("清单 metadata.version 不能为空".to_string());
    }
    if manifest.spec.images.is_empty() {
        return Err("清单 spec.images 不能为空".to_string());
    }
    normalize_registry(&manifest.spec.registry)?;
    if let Some(namespace) = manifest.spec.namespace.as_deref() {
        validate_repository_path(namespace, "namespace")?;
    }

    let mut names = HashSet::new();
    for (index, image) in manifest.spec.images.iter().enumerate() {
        if image.name.trim().is_empty() {
            return Err(format!("第 {} 个镜像缺少 name", index + 1));
        }
        if !names.insert(image.name.trim().to_string()) {
            return Err(format!("镜像名称重复: {}", image.name));
        }
        validate_repository_path(&image.repository, "repository")?;
        let digest = image
            .digest
            .as_deref()
            .ok_or_else(|| format!("镜像 {} 缺少 digest，必须使用 sha256 摘要", image.name))?;
        if !is_valid_digest(digest) {
            return Err(format!(
                "镜像 {} 的 digest 无效，必须形如 sha256:<64位小写十六进制>",
                image.name
            ));
        }
        if let Some(tag) = image.tag.as_deref() {
            if tag.trim().is_empty() || tag.contains(' ') || tag.contains('@') {
                return Err(format!("镜像 {} 的 tag 无效", image.name));
            }
        }
        if let Some(platform) = image.platform.as_deref() {
            validate_platform(platform)?;
        }
    }
    if let Some(platform) = manifest.spec.platform.as_deref() {
        validate_platform(platform)?;
    }
    Ok(())
}

fn resolve_images(manifest: &ImageManifest) -> Result<Vec<ResolvedImage>, String> {
    let registry = normalize_registry(&manifest.spec.registry)?;
    let namespace = manifest
        .spec
        .namespace
        .as_deref()
        .map(|value| value.trim_matches('/'))
        .filter(|value| !value.is_empty());
    let default_platform = manifest.spec.platform.clone();

    let mut resolved = Vec::with_capacity(manifest.spec.images.len());
    for image in &manifest.spec.images {
        let digest = image.digest.clone().unwrap_or_default();
        let repository = image.repository.trim().trim_matches('/');
        let path = match namespace {
            Some(ns) => format!("{ns}/{repository}"),
            None => repository.to_string(),
        };
        let reference = format!("{registry}/{path}@{digest}");
        resolved.push(ResolvedImage {
            name: image.name.trim().to_string(),
            repository: path,
            reference,
            digest,
            platform: image.platform.clone().or_else(|| default_platform.clone()),
            size_bytes: image.size_bytes.unwrap_or(0),
        });
    }
    Ok(resolved)
}

fn manifest_warnings(manifest: &ImageManifest, images: &[ResolvedImage]) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(min_version) = manifest.metadata.min_tool_version.as_deref() {
        if version_lt(env!("CARGO_PKG_VERSION"), min_version) {
            warnings.push(format!(
                "清单要求工具版本不低于 {min_version}，当前为 {}",
                env!("CARGO_PKG_VERSION")
            ));
        }
    }
    if images.iter().any(|image| image.size_bytes == 0) {
        warnings.push("部分镜像未提供 sizeBytes，无法预估下载体积".to_string());
    }
    if manifest.spec.signature.is_none() {
        warnings.push("清单未包含 signature，建议在正式发布时附加签名".to_string());
    }
    if manifest.spec.checksums.is_none() {
        warnings.push("清单未包含 checksums，建议附加校验和文件".to_string());
    }
    if images.iter().all(|image| image.platform.is_none()) {
        warnings.push("清单未指定 platform，将按目标机默认架构拉取".to_string());
    }
    warnings
}

fn normalize_registry(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if without_scheme.is_empty() {
        return Err("清单 spec.registry 不能为空".to_string());
    }
    if without_scheme.contains('/')
        || without_scheme.contains('@')
        || without_scheme.chars().any(|c| {
            !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        })
    {
        return Err(format!("镜像仓库地址无效: {raw}"));
    }
    Ok(without_scheme.to_ascii_lowercase())
}

fn validate_repository_path(value: &str, field: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.contains("//")
        || trimmed.contains("..")
        || trimmed.contains('@')
        || trimmed.contains(':')
        || trimmed.chars().any(|c| {
            !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
        })
    {
        return Err(format!("清单 {field} 无效: {value}"));
    }
    Ok(())
}

fn validate_platform(value: &str) -> Result<(), String> {
    let parts: Vec<&str> = value.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| part.is_empty())
        || parts.iter().any(|part| {
            !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        })
    {
        return Err(format!("平台格式无效: {value}，应形如 linux/amd64"));
    }
    Ok(())
}

fn is_valid_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

fn version_lt(current: &str, required: &str) -> bool {
    crate::core::version::compare(current, required) == std::cmp::Ordering::Less
}

fn select_images<'a>(
    summary: &'a ManifestSummary,
    requested: &[String],
) -> Result<Vec<&'a ResolvedImage>, String> {
    if requested.is_empty() {
        return Ok(summary.images.iter().collect());
    }
    let mut selected = Vec::new();
    for name in requested {
        let image = summary
            .images
            .iter()
            .find(|image| image.name == *name)
            .ok_or_else(|| format!("清单中不存在镜像: {name}"))?;
        selected.push(image);
    }
    Ok(selected)
}

fn pull_remote_blocking(
    window: &tauri::Window,
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<String>,
    summary: ManifestSummary,
    username: Option<String>,
    password: Option<String>,
    image_names: Vec<String>,
    export_dir: Option<String>,
) -> Result<Vec<ImagePullResult>, String> {
    if host.trim().is_empty() {
        return Err("请先在连接配置中填写目标主机".to_string());
    }
    let selected = select_images(&summary, &image_names)?;
    let key = resolve_existing_key(key_path, host)?;
    let registry = normalize_registry(&summary.manifest.spec.registry)?;

    let login_script = remote_login_script(&registry, username.as_deref(), password.as_deref())?;

    let total = selected.len();
    let mut results = Vec::with_capacity(total);
    for (index, image) in selected.iter().enumerate() {
        emit_progress(
            window,
            "pull",
            index + 1,
            total,
            &image.name,
            0,
            &format!("正在拉取 [{} / {}] {}", index + 1, total, image.name),
            false,
        );

        let pull_script = if let Some(platform) = image.platform.as_deref() {
            format!(
                "docker pull --platform {} {}\n",
                shell_quote(platform),
                shell_quote(&image.reference)
            )
        } else {
            format!("docker pull {}\n", shell_quote(&image.reference))
        };
        let script = format!("set -e\n{login_script}{pull_script}");
        let output = run_ssh_script(host, port, user, key.as_deref(), &script)?;
        if !output.status.success() {
            return Err(format!("拉取镜像 {} 失败，请检查镜像摘要、仓库权限、目标机网络和 Docker 状态", image.name));
        }

        verify_remote_digest(host, port, user, key.as_deref(), image)?;

        if let Some(dir) = export_dir.as_deref().map(str::trim).filter(|dir| !dir.is_empty()) {
            let file_name = format!("{}.tar", safe_file_name(&image.name));
            let target = format!("{}/{}", dir.trim_end_matches('/'), file_name);
            let script = format!(
                "set -e\nmkdir -p {}\ndocker save -o {} {}\nstat -c '%s' {}\n",
                shell_quote(dir),
                shell_quote(&target),
                shell_quote(&image.reference),
                shell_quote(&target)
            );
            let output = run_ssh_script(host, port, user, key.as_deref(), &script)?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(format!("导出镜像 {} 失败: {stderr}", image.name));
            }
            let size = String::from_utf8_lossy(&output.stdout)
                .lines()
                .last()
                .and_then(|line| line.trim().parse::<u64>().ok())
                .unwrap_or(0);
            emit_progress(
                window,
                "export",
                index + 1,
                total,
                &image.name,
                100,
                &format!("已导出 {}", target),
                false,
            );
            results.push(ImagePullResult {
                name: image.name.clone(),
                reference: image.reference.clone(),
                digest: image.digest.clone(),
                platform: image.platform.clone(),
                exported_path: Some(target),
                size_bytes: size,
                message: "已拉取并导出".to_string(),
            });
        } else {
            results.push(ImagePullResult {
                name: image.name.clone(),
                reference: image.reference.clone(),
                digest: image.digest.clone(),
                platform: image.platform.clone(),
                exported_path: None,
                size_bytes: image.size_bytes,
                message: "已拉取".to_string(),
            });
        }

        emit_progress(
            window,
            "pull",
            index + 1,
            total,
            &image.name,
            100,
            &format!("{} 完成", image.name),
            false,
        );
    }

    emit_progress(
        window,
        "done",
        total,
        total,
        "ALL",
        100,
        "镜像清单处理完成",
        true,
    );
    Ok(results)
}

fn remote_login_script(
    registry: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<String, String> {
    let Some(username) = username.map(str::trim).filter(|value| !value.is_empty()) else {
        if password.is_some_and(|value| !value.is_empty()) {
            return Err("已填写仓库密码，请同时填写仓库用户名".to_string());
        }
        return Ok(String::new());
    };
    let password = password.unwrap_or_default();
    if password.is_empty() {
        return Err("已填写仓库用户名，请同时填写密码".to_string());
    }
    Ok(format!(
        "umask 077\nDOCKER_CONFIG=$(mktemp -d)\nexport DOCKER_CONFIG\ntrap 'rm -rf \"$DOCKER_CONFIG\"' EXIT\nprintf '%s' {} | docker login {} -u {} --password-stdin >/dev/null\n",
        shell_quote(password),
        shell_quote(registry),
        shell_quote(username)
    ))
}

fn verify_remote_digest(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    image: &ResolvedImage,
) -> Result<(), String> {
    let script = format!(
        "docker image inspect {} --format '{{{{range .RepoDigests}}}}{{{{println .}}}}{{{{end}}}}' 2>/dev/null || true",
        shell_quote(&image.reference)
    );
    let output = run_ssh_script(host, port, user, key, &script)?;
    let digests = String::from_utf8_lossy(&output.stdout);
    let expected = format!("@{}", image.digest);
    if digests.trim().is_empty() {
        return Ok(());
    }
    if !digests.lines().any(|line| line.trim().ends_with(&expected)) {
        return Err(format!(
            "镜像 {} 的 digest 校验失败，期望 {}",
            image.name, image.digest
        ));
    }
    Ok(())
}

fn pull_local_blocking(
    window: &tauri::Window,
    summary: ManifestSummary,
    username: Option<String>,
    password: Option<String>,
    image_names: Vec<String>,
    export_dir: Option<String>,
) -> Result<Vec<ImagePullResult>, String> {
    let selected = select_images(&summary, &image_names)?;
    let registry = normalize_registry(&summary.manifest.spec.registry)?;

    let config = login_local(&registry, username, password)?;

    let docker = find_binary("docker");
    let total = selected.len();
    let mut results = Vec::with_capacity(total);
    for (index, image) in selected.iter().enumerate() {
        emit_progress(
            window,
            "pull",
            index + 1,
            total,
            &image.name,
            0,
            &format!("正在拉取 [{} / {}] {}", index + 1, total, image.name),
            false,
        );

        let mut args = vec!["pull".to_string()];
        if let Some(platform) = image.platform.as_deref() {
            args.push("--platform".to_string());
            args.push(platform.to_string());
        }
        args.push(image.reference.clone());
        run_local(&docker, &args, config.as_ref().map(|config| config.path.as_path()))
            .map_err(|_| format!("拉取镜像 {} 失败，请检查镜像摘要、仓库权限和网络", image.name))?;

        verify_local_digest(&docker, image)?;

        if let Some(dir) = export_dir.as_deref().map(str::trim).filter(|dir| !dir.is_empty()) {
            fs::create_dir_all(dir).map_err(|e| format!("创建导出目录失败: {e}"))?;
            let file_name = format!("{}.tar", safe_file_name(&image.name));
            let target = PathBuf::from(dir).join(file_name);
            run_local(
                &docker,
                &[
                    "save".to_string(),
                    "-o".to_string(),
                    target.display().to_string(),
                    image.reference.clone(),
                ],
                None,
            )
            .map_err(|e| format!("导出镜像 {} 失败: {e}", image.name))?;
            let size = fs::metadata(&target).map(|meta| meta.len()).unwrap_or(0);
            emit_progress(
                window,
                "export",
                index + 1,
                total,
                &image.name,
                100,
                &format!("已导出 {}", target.display()),
                false,
            );
            results.push(ImagePullResult {
                name: image.name.clone(),
                reference: image.reference.clone(),
                digest: image.digest.clone(),
                platform: image.platform.clone(),
                exported_path: Some(target.display().to_string()),
                size_bytes: size,
                message: "已拉取并导出".to_string(),
            });
        } else {
            results.push(ImagePullResult {
                name: image.name.clone(),
                reference: image.reference.clone(),
                digest: image.digest.clone(),
                platform: image.platform.clone(),
                exported_path: None,
                size_bytes: image.size_bytes,
                message: "已拉取".to_string(),
            });
        }

        emit_progress(
            window,
            "pull",
            index + 1,
            total,
            &image.name,
            100,
            &format!("{} 完成", image.name),
            false,
        );
    }

    emit_progress(
        window,
        "done",
        total,
        total,
        "ALL",
        100,
        "镜像清单处理完成",
        true,
    );
    Ok(results)
}

fn login_local(
    registry: &str,
    username: Option<String>,
    password: Option<String>,
) -> Result<Option<TempDockerConfig>, String> {
    let Some(username) = username.as_deref().map(str::trim).filter(|value| !value.is_empty()) else {
        if password.as_deref().is_some_and(|value| !value.is_empty()) {
            return Err("已填写仓库密码，请同时填写仓库用户名".to_string());
        }
        return Ok(None);
    };
    let password = password.unwrap_or_default();
    if password.is_empty() {
        return Err("已填写仓库用户名，请同时填写密码".to_string());
    }
    let config = TempDockerConfig::create()?;
    login_docker(registry, username, &password, &config.path)?;
    Ok(Some(config))
}

fn verify_local_digest(docker: &str, image: &ResolvedImage) -> Result<(), String> {
    let output = Command::new(docker)
        .args([
            "image",
            "inspect",
            &image.reference,
            "--format",
            "{{range .RepoDigests}}{{println .}}{{end}}",
        ])
        .output()
        .map_err(|e| format!("校验镜像 digest 失败: {e}"))?;
    let digests = String::from_utf8_lossy(&output.stdout);
    let expected = format!("@{}", image.digest);
    if digests.trim().is_empty() {
        return Ok(());
    }
    if !digests.lines().any(|line| line.trim().ends_with(&expected)) {
        return Err(format!(
            "镜像 {} 的 digest 校验失败，期望 {}",
            image.name, image.digest
        ));
    }
    Ok(())
}

fn run_local(docker: &str, args: &[String], config: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new(docker);
    command.args(args);
    if let Some(config) = config {
        command.env("DOCKER_CONFIG", config);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|e| format!("启动 docker 命令失败: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 docker 命令失败: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "docker 命令执行失败".to_string()
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn safe_file_name(name: &str) -> String {
    let mut value: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if value.is_empty() || value == "." || value == ".." {
        value = "image".to_string();
    }
    value
}

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    window: &tauri::Window,
    phase: &str,
    current: usize,
    total: usize,
    image: &str,
    percent: usize,
    message: &str,
    finished: bool,
) {
    let _ = window.emit(
        "registry-progress",
        RegistryProgress {
            phase: phase.to_string(),
            current,
            total,
            image: image.to_string(),
            percent,
            message: message.to_string(),
            finished,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> ImageManifest {
        serde_yaml::from_str(
            r#"
apiVersion: deploy.desktop/v1
kind: ImageBundle
metadata:
  name: demo
  version: 1.2.3
spec:
  registry: registry.cn-hangzhou.aliyuncs.com
  namespace: demo
  platform: linux/amd64
  images:
    - name: backend
      repository: backend
      digest: sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
      sizeBytes: 1024
"#,
        )
        .expect("manifest should parse")
    }

    #[test]
    fn accepts_valid_manifest() {
        let manifest = sample_manifest();
        validate_manifest(&manifest).expect("manifest should be valid");
        let images = resolve_images(&manifest).expect("images should resolve");
        assert_eq!(
            images[0].reference,
            "registry.cn-hangzhou.aliyuncs.com/demo/backend@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(images[0].platform.as_deref(), Some("linux/amd64"));
    }

    #[test]
    fn rejects_missing_digest() {
        let mut manifest = sample_manifest();
        manifest.spec.images[0].digest = None;
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_invalid_digest() {
        let mut manifest = sample_manifest();
        manifest.spec.images[0].digest = Some("sha256:xyz".to_string());
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_duplicate_image_names() {
        let mut manifest = sample_manifest();
        let image = manifest.spec.images[0].clone();
        manifest.spec.images.push(image);
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_relative_repository_path() {
        let mut manifest = sample_manifest();
        manifest.spec.images[0].repository = "../backend".to_string();
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn compares_versions() {
        assert!(version_lt("0.1.0", "0.2.0"));
        assert!(!version_lt("1.2.0", "1.2.0"));
        assert!(!version_lt("1.3.0", "1.2.9"));
    }

    #[test]
    fn selected_source_rewrites_only_registry_and_optional_namespace() {
        let manifest = sample_manifest();
        let original_digest = manifest.spec.images[0].digest.clone();
        let summary = ManifestSummary {
            images: resolve_images(&manifest).unwrap(), manifest, source: "manifest.yaml".to_string(), total_size_bytes: 1024, warnings: Vec::new(),
        };
        let mut access = RegistrySourceAccess {
            source: super::super::registry_sources::RegistrySource { address: "private.example.com:5000".to_string(), ..Default::default() }, password: None,
        };
        let resolved = resolve_with_source(summary, Some(&access)).unwrap();
        assert!(resolved.images[0].reference.starts_with("private.example.com:5000/demo/backend@sha256:"));
        access.source.namespace = "mirror/project".to_string();
        let resolved = resolve_with_source(resolved, Some(&access)).unwrap();
        assert!(resolved.images[0].reference.starts_with("private.example.com:5000/mirror/project/backend@sha256:"));
        assert_eq!(resolved.manifest.spec.images[0].digest, original_digest);
        assert_eq!(resolved.images[0].platform.as_deref(), Some("linux/amd64"));
    }

    #[test]
    fn remote_login_uses_disposable_config_and_rejects_incomplete_credentials() {
        let script = remote_login_script("registry.example.com", Some("reader"), Some("test'password")).unwrap();
        assert!(script.contains("DOCKER_CONFIG=$(mktemp -d)"));
        assert!(script.contains("trap 'rm -rf \"$DOCKER_CONFIG\"' EXIT"));
        assert!(script.contains("--password-stdin"));
        assert!(script.contains(&shell_quote("test'password")));
        assert!(remote_login_script("registry.example.com", None, Some("test-password")).is_err());
        assert!(remote_login_script("registry.example.com", Some("reader"), None).is_err());
    }
}
