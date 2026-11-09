use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use super::bundle::sha256_hex;
use super::deployment_review::{check_local_endpoint, valid_image_reference};
use super::docker_setup::{docker_command, docker_path_env};
use super::deployment_image_bindings::{image_identity, DeploymentImageBinding};
use super::image_library::{
    docker_failure, export_images_blocking, login_docker, redact_secret, registry_for_reference,
    ImageExportResult, ImageLibraryProgress, TempDockerConfig,
};
use super::registry_sources::{source_access, source_credentials, RegistrySourceAccess};
use super::remote_ssh::{
    base64_encode, ensure_known_host, resolve_existing_key, run_ssh_script,
    run_ssh_script_strict_with_timeout, run_ssh_script_with_timeout, shell_quote,
};
use super::remote_update::{compose_service_views, resolve_bundle_file};
use crate::core::process::{self, Input, Options};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentImagePullResult {
    pub reference: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentPullCredential {
    pub registry_source_id: String,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeploymentImageTransferResult {
    #[serde(flatten)]
    pub export: ImageExportResult,
    pub binding: Option<DeploymentImageBinding>,
}

#[derive(Debug, Clone, PartialEq)]
struct PullLogin {
    registry: String,
    username: String,
    password: String,
}

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
        let declared = declared_images(&bundle_dir)?;
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
            return Err(String::from("无法确认目标主机的 Docker 架构，请检查连接和 Docker 后重试"));
        }
        let target = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        for reference in &references {
            let mut command = docker_command();
            command.args(["image", "inspect", "--", reference]);
            let output = process::run(command, Input::Bytes(vec![]), Options::default())?;
            if !output.status.success() {
                return Err(format!("本机缺少镜像 {reference} 或 Docker 不可用，请先在镜像管理中准备该镜像，或选择离线镜像包"));
            }
            let images: Vec<Value> = serde_json::from_slice(&output.stdout)
                .map_err(|_| String::from("无法确认本机镜像信息，请检查 Docker 后重试"))?;
            let image = images.first()
                .ok_or_else(|| String::from("未读取到本机镜像信息，请重新准备镜像"))?;
            validate_platform(image, &target, reference)?;
        }
        export_images_blocking(&window, &app, references)
    })
    .await
    .map_err(|_| String::from("导出镜像任务未完成，请检查本机 Docker 后重试"))?
}

#[tauri::command]
pub async fn pull_deployment_images(
    window: tauri::Window,
    app: AppHandle,
    bundle_dir: String,
    references: Vec<String>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
    password: Option<String>,
    registry_source_id: Option<String>,
    registry_credentials: Option<Vec<DeploymentPullCredential>>,
    install_dir: Option<String>,
) -> Result<Vec<DeploymentImagePullResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let references = dedupe_references(references);
        let declared = declared_images(&bundle_dir)?;
        validate_selection(&references, &declared)?;
        let logins = resolve_pull_logins(
            &app,
            registry_source_id.as_deref(),
            password,
            registry_credentials,
            &references,
        )?;
        let host = host.unwrap_or_default();
        let host = host.trim();
        let user = user.unwrap_or_default();
        let mut on_confirmed = |reference: &str| {
            if reference.contains("@sha256:") {
                if let Some(install) = install_dir.as_deref().filter(|value| !value.trim().is_empty()) {
                    super::deployment_image_bindings::clear_bindings(
                        install,
                        &[reference.to_owned()],
                        if host.is_empty() { None } else { Some(host) },
                        port.unwrap_or(22),
                        user.trim(),
                        key_path.clone(),
                    )?;
                }
            }
            Ok(())
        };
        let results = if host.is_empty() {
            pull_local_blocking(&window, &app, &references, &logins, &mut on_confirmed)?
        } else {
            if user.trim().is_empty() {
                return Err(String::from("请先填写目标主机登录用户"));
            }
            pull_remote_blocking(
                &window,
                host,
                port.unwrap_or(22),
                user.trim(),
                key_path.clone(),
                &references,
                &logins,
                &mut on_confirmed,
            )?
        };
        Ok(results)
    })
    .await
    .map_err(|_| String::from("拉取镜像任务未完成，请检查目标环境后重试"))?
}

#[tauri::command]
pub async fn prepare_local_deployment_images(
    window: tauri::Window,
    app: AppHandle,
    bundle_dir: String,
    references: Vec<String>,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    registry_credentials: Option<Vec<DeploymentPullCredential>>,
) -> Result<Vec<DeploymentImageTransferResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let references = dedupe_references(references);
        let declared = declared_images(&bundle_dir)?;
        validate_selection(&references, &declared)?;
        let logins = resolve_pull_logins(&app, None, None, registry_credentials, &references)?;
        prepare_local_for_transfer(
            &window,
            &app,
            &host,
            port,
            &user,
            key_path,
            &references,
            &logins,
        )
    })
    .await
    .map_err(|_| String::from("准备本机镜像任务未完成，请检查本机 Docker 后重试"))?
}

pub(super) fn declared_images(bundle_dir: &str) -> Result<Vec<String>, String> {
    let root = std::fs::canonicalize(bundle_dir.trim())
        .map_err(|_| String::from("无法访问交付包，请重新选择制品"))?;
    let bundle = super::bundle::Bundle::load(&root)?;
    bundle.ensure_compatible()?;
    let compose = resolve_bundle_file(&root, bundle.effective_compose())?;
    let content = std::fs::read_to_string(compose)
        .map_err(|_| String::from("无法读取部署清单，请重新校验交付包"))?;
    let declared: Vec<String> = compose_service_views(&content)
        .map_err(|_| String::from("部署清单格式有误，请检查交付包"))?
        .into_iter()
        .map(|service| service.image)
        .collect();
    Ok(declared)
}

fn dedupe_references(references: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    references
        .into_iter()
        .map(|reference| reference.trim().to_owned())
        .filter(|reference| !reference.is_empty() && seen.insert(reference.clone()))
        .collect()
}

fn normalize_registry_host(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "registry-1.docker.io" | "index.docker.io" => String::from("docker.io"),
        _ => value,
    }
}

fn resolve_pull_logins(
    app: &AppHandle,
    registry_source_id: Option<&str>,
    legacy_password: Option<String>,
    registry_credentials: Option<Vec<DeploymentPullCredential>>,
    references: &[String],
) -> Result<HashMap<String, PullLogin>, String> {
    let mut requested = registry_credentials.unwrap_or_default();
    if requested.is_empty() {
        match registry_source_id.filter(|id| !id.trim().is_empty()) {
            Some(id) => requested.push(DeploymentPullCredential {
                registry_source_id: id.to_string(),
                password: legacy_password,
            }),
            None if legacy_password
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()) =>
            {
                return Err(String::from("请选择登录镜像源后再填写密码"));
            }
            None => {}
        }
    }
    let reference_registries: HashSet<String> = references
        .iter()
        .map(|reference| normalize_registry_host(&registry_for_reference(reference)))
        .collect();
    let mut logins: HashMap<String, PullLogin> = HashMap::new();
    for credential in requested {
        if credential.registry_source_id.trim().is_empty() {
            continue;
        }
        let Some(access) = source_access(app, Some(credential.registry_source_id.trim()))? else {
            continue;
        };
        add_pull_login(
            &mut logins,
            access,
            credential.password,
            &reference_registries,
        )?;
    }
    Ok(logins)
}

fn add_pull_login(
    logins: &mut HashMap<String, PullLogin>,
    access: RegistrySourceAccess,
    password: Option<String>,
    reference_registries: &HashSet<String>,
) -> Result<(), String> {
    let registry = normalize_registry_host(&access.source.address);
    if !reference_registries.contains(&registry) {
        return Err(format!(
            "所选镜像源 {} 与待处理镜像的仓库地址不一致，请选择匹配的镜像源",
            access.source.address
        ));
    }
    if access.source.username.trim().is_empty() {
        if password
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(String::from("所选镜像源未配置用户名，无法使用密码登录"));
        }
        return Ok(());
    }
    let (username, password) = source_credentials(access, password)?;
    let Some(username) = username else {
        return Ok(());
    };
    let login = PullLogin {
        registry: registry.clone(),
        username,
        password: password.unwrap_or_default(),
    };
    if let Some(existing) = logins.get(&registry) {
        if existing != &login {
            return Err(format!(
                "同一镜像仓库 {registry} 选择了多组不同凭据，请只保留一组"
            ));
        }
    } else {
        logins.insert(registry, login);
    }
    Ok(())
}

fn login_for_reference<'a>(
    reference: &str,
    logins: &'a HashMap<String, PullLogin>,
) -> Option<&'a PullLogin> {
    logins.get(&normalize_registry_host(&registry_for_reference(reference)))
}

fn pull_local_blocking(
    window: &tauri::Window,
    app: &AppHandle,
    references: &[String],
    logins: &HashMap<String, PullLogin>,
    on_confirmed: &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<Vec<DeploymentImagePullResult>, String> {
    check_local_endpoint()?;
    let config = if logins.is_empty() {
        None
    } else {
        let config = TempDockerConfig::create_for_current_context()?;
        for login in logins.values() {
            login_docker(
                &login.registry,
                &login.username,
                &login.password,
                &config.path,
            )?;
        }
        Some(config)
    };
    let target_platform = local_docker_platform(config.as_ref())?;
    let total = references.len();
    let mut results = Vec::with_capacity(total);
    for (index, reference) in references.iter().enumerate() {
        emit_pull_progress(
            window,
            index * 100 / total,
            index + 1,
            total,
            reference,
            &format!("正在拉取 [{} / {}] {reference}", index + 1, total),
        );
        let login = login_for_reference(reference, logins);
        let command = local_pull_command(reference, &target_platform);
        let output = run_local_docker(
            command,
            if login.is_some() {
                config.as_ref()
            } else {
                None
            },
            Duration::from_secs(1800),
            &format!("拉取镜像 {reference}"),
        )?;
        let mut message = String::from("已拉取并确认");
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let mut empty_error = stderr.is_empty();
            let mut error_text = if stderr.is_empty() {
                stdout
            } else {
                stderr
            };
            let mut retried = false;
            if empty_error {
                let mut retry = docker_command();
                retry.args(["pull", reference]);
                match run_local_docker(
                    retry,
                    if login.is_some() {
                        config.as_ref()
                    } else {
                        None
                    },
                    Duration::from_secs(600),
                    &format!("重新拉取镜像 {reference}"),
                ) {
                    Ok(output) => {
                        retried = output.status.success();
                        if !retried {
                            let retry_stderr =
                                String::from_utf8_lossy(&output.stderr).trim().to_owned();
                            let retry_stdout =
                                String::from_utf8_lossy(&output.stdout).trim().to_owned();
                            empty_error = retry_stderr.is_empty();
                            if !retry_stderr.is_empty() {
                                error_text = retry_stderr;
                            } else if !retry_stdout.is_empty() {
                                error_text = retry_stdout;
                            }
                        }
                    }
                    Err(error) => {
                        empty_error = false;
                        error_text = error;
                    }
                }
            }
            if retried {
                message = String::from("已重新拉取并确认");
            } else {
                let mirror = if login.is_none()
                    && is_docker_hub_tag(reference)
                    && (empty_error || registry_connection_failure(&error_text))
                {
                    Some(pull_via_docker_hub_mirror(
                        window,
                        app,
                        reference,
                        &target_platform,
                        index,
                        total,
                    ))
                } else {
                    None
                };
                match mirror {
                    Some(Ok(source)) => {
                        message = format!("已通过 {source} 加速源拉取并保存为原镜像地址")
                    }
                    outcome => {
                        let message = pull_failure(&error_text, login.is_some());
                        let message = redact_secret(
                            &message,
                            login
                                .map(|value| value.password.as_str())
                                .unwrap_or_default(),
                        );
                        let message = match outcome {
                            Some(Err(detail)) => {
                                format!("{message}；加速源回退未完成：{detail}")
                            }
                            _ => message,
                        };
                        return Err(format!("拉取镜像 {reference} 失败：{message}"));
                    }
                }
            }
        }
        let image_platform = inspect_local_pull(reference, config.as_ref())?;
        if !platform_matches(&image_platform, &target_platform) {
            return Err(format!(
                "本机镜像 {reference} 的平台 {image_platform} 与本机 Docker {target_platform} 不一致，请使用对应架构的镜像"
            ));
        }
        on_confirmed(reference)?;
        emit_pull_progress(
            window,
            (index + 1) * 100 / total,
            index + 1,
            total,
            reference,
            &format!("已拉取 {reference}"),
        );
        results.push(DeploymentImagePullResult {
            reference: reference.clone(),
            message,
        });
    }
    Ok(results)
}

fn pull_via_docker_hub_mirror(
    window: &tauri::Window,
    app: &AppHandle,
    reference: &str,
    target_platform: &str,
    index: usize,
    total: usize,
) -> Result<String, String> {
    if docker_hub_auth_configured() {
        log::warn!(
            target: "deployment",
            "检测到 Docker Hub 登录配置，跳过镜像 {reference} 的自动加速源回退"
        );
        return Err(String::from(
            "检测到 Docker Hub 登录配置，未自动切换加速源",
        ));
    }
    let expected_hub_digest = match docker_hub_manifest_digest(reference) {
        HubManifest::Digest(digest) => Some(digest),
        HubManifest::Denied => {
            log::warn!(
                target: "deployment",
                "Docker Hub 无法读取镜像 {reference}，可能已私有化或不存在，停止自动回退"
            );
            return Err(String::from(
                "Docker Hub 无法读取该镜像，可能已私有化或不存在，未自动切换加速源",
            ));
        }
        HubManifest::Unavailable => {
            log::warn!(
                target: "deployment",
                "Docker Hub 不可达，按加速源清单摘要校验镜像 {reference}"
            );
            None
        }
    };
    let mirrors = super::registry_sources::docker_hub_accelerators(app)
        .map_err(|_| String::from("无法读取镜像源配置"))?;
    if mirrors.is_empty() {
        return Err(String::from(
            "未找到可用的预置加速源，请在设置中保留加速源后重试",
        ));
    }
    let mut attempts = Vec::new();
    for source in mirrors {
        let Some((repository, tag)) =
            docker_hub_mirror_repository(reference, &source.namespace)
        else {
            continue;
        };
        let digest = match mirror_manifest_digest(&source.address, &repository, &tag) {
            Ok(digest) => digest,
            Err(error) => {
                log::warn!(
                    target: "deployment",
                    "无法读取加速源 {} 的镜像清单 {repository}:{tag}：{error}",
                    source.address
                );
                attempts.push(format!("{}：{error}", source.name));
                continue;
            }
        };
        if let Some(expected) = expected_hub_digest.as_deref() {
            if digest != expected {
                log::warn!(
                    target: "deployment",
                    "加速源 {} 的镜像清单摘要 {digest} 与 Docker Hub {expected} 不一致",
                    source.address
                );
                attempts.push(format!("{}：镜像摘要与 Docker Hub 不一致", source.name));
                continue;
            }
        } else {
            log::warn!(
                target: "deployment",
                "Docker Hub 不可达，按加速源 {} 的清单摘要 {digest} 校验",
                source.address
            );
        }
        emit_pull_progress(
            window,
            index * 100 / total,
            index + 1,
            total,
            reference,
            &format!("直连镜像仓库失败，正在通过 {} 加速拉取 {reference}", source.name),
        );
        log::warn!(
            target: "deployment",
            "镜像 {reference} 直连失败，尝试通过加速源 {} 拉取摘要 {digest}",
            source.address,
        );
        let mirror = format!("{}/{repository}@{digest}", source.address);
        let pull = local_pull_command(&mirror, target_platform);
        let Ok(output) = run_local_docker(
            pull,
            None,
            Duration::from_secs(600),
            &format!("通过 {} 拉取镜像 {reference}", source.name),
        ) else {
            attempts.push(format!("{}：拉取命令未完成", source.name));
            continue;
        };
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            let detail = detail.trim();
            attempts.push(format!(
                "{}：{}",
                source.name,
                if detail.is_empty() {
                    "未返回错误信息"
                } else {
                    detail
                }
            ));
            continue;
        }
        let platform = match inspect_local_image(&mirror, None) {
            Ok(Some(platform)) => platform,
            _ => {
                remove_local_image(&mirror);
                attempts.push(format!("{}：无法确认镜像平台", source.name));
                continue;
            }
        };
        if !platform_matches(&platform, target_platform) {
            log::warn!(
                target: "deployment",
                "加速源 {} 返回的镜像 {mirror} 平台为 {platform}，与目标 {target_platform} 不一致",
                source.address
            );
            remove_local_image(&mirror);
            attempts.push(format!("{}：镜像平台与目标不一致", source.name));
            continue;
        }
        let Some(image_id) = local_image_id(&mirror) else {
            remove_local_image(&mirror);
            attempts.push(format!("{}：无法读取镜像标识", source.name));
            continue;
        };
        let mut tag = docker_command();
        tag.args(["tag", &image_id, reference]);
        let Ok(output) = run_local_docker(
            tag,
            None,
            Duration::from_secs(60),
            &format!("保存镜像 {reference} 的原地址"),
        ) else {
            remove_local_image(&mirror);
            attempts.push(format!("{}：保存原镜像地址未完成", source.name));
            continue;
        };
        if !output.status.success() {
            remove_local_image(&mirror);
            attempts.push(format!("{}：保存原镜像地址失败", source.name));
            continue;
        }
        log::info!(
            target: "deployment",
            "已通过加速源 {} 拉取镜像 {reference}，摘要 {digest}",
            source.address,
        );
        remove_local_image(&mirror);
        return Ok(source.name);
    }
    if attempts.is_empty() {
        Err(String::from("没有可用的加速源"))
    } else {
        Err(attempts
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join("；"))
    }
}

fn is_docker_hub_tag(reference: &str) -> bool {
    !reference.contains("@sha256:") && docker_hub_path(reference).is_some()
}

fn docker_hub_mirror_repository(reference: &str, namespace: &str) -> Option<(String, String)> {
    let path = docker_hub_path(reference)?;
    if path.is_empty() || path.contains('@') {
        return None;
    }
    let (name, tag) = match path.rsplit_once(':') {
        Some((name, tag)) if !tag.is_empty() && !tag.contains('/') => (name, tag),
        _ => (path, "latest"),
    };
    if name.is_empty() {
        return None;
    }
    let namespace = namespace.trim().trim_matches('/');
    let repository = if name.contains('/') || namespace.is_empty() {
        name.to_string()
    } else {
        format!("{namespace}/{name}")
    };
    Some((repository, tag.to_string()))
}

fn docker_hub_path(reference: &str) -> Option<&str> {
    let lower = reference.to_ascii_lowercase();
    for prefix in [
        "docker.io/",
        "index.docker.io/",
        "registry-1.docker.io/",
    ] {
        if lower.starts_with(prefix) {
            return reference.get(prefix.len()..);
        }
    }
    if normalize_registry_host(&registry_for_reference(reference)) == "docker.io" {
        Some(reference)
    } else {
        None
    }
}

fn docker_hub_repository_and_tag(reference: &str) -> Option<(String, String)> {
    let path = docker_hub_path(reference)?.trim();
    if path.is_empty() || path.contains('@') {
        return None;
    }
    let (name, tag) = match path.rsplit_once(':') {
        Some((name, tag)) if !tag.is_empty() && !tag.contains('/') => (name, tag),
        _ => (path, "latest"),
    };
    if name.is_empty() {
        return None;
    }
    let repository = if name.contains('/') {
        name.to_string()
    } else {
        format!("library/{name}")
    };
    Some((repository, tag.to_string()))
}

enum HubManifest {
    Digest(String),
    Denied,
    Unavailable,
}

fn docker_hub_manifest_digest(reference: &str) -> HubManifest {
    let Some((repository, tag)) = docker_hub_repository_and_tag(reference) else {
        return HubManifest::Denied;
    };
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
    else {
        return HubManifest::Unavailable;
    };
    let scope = format!("repository:{repository}:pull");
    let Ok(token_response) = client
        .get("https://auth.docker.io/token")
        .query(&[("service", "registry.docker.io"), ("scope", scope.as_str())])
        .send()
    else {
        return HubManifest::Unavailable;
    };
    if matches!(
        token_response.status(),
        reqwest::StatusCode::UNAUTHORIZED
            | reqwest::StatusCode::FORBIDDEN
            | reqwest::StatusCode::NOT_FOUND
    ) {
        return HubManifest::Denied;
    }
    if !token_response.status().is_success() {
        return HubManifest::Unavailable;
    }
    let Ok(token) = serde_json::from_slice::<Value>(
        &token_response.bytes().unwrap_or_default(),
    ) else {
        return HubManifest::Unavailable;
    };
    let Some(token) = token.get("token").and_then(Value::as_str) else {
        return HubManifest::Unavailable;
    };
    let Ok(response) = client
        .head(format!(
            "https://registry-1.docker.io/v2/{repository}/manifests/{tag}"
        ))
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}"),
        )
        .header(
            reqwest::header::ACCEPT,
            "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json",
        )
        .send()
    else {
        return HubManifest::Unavailable;
    };
    if matches!(
        response.status(),
        reqwest::StatusCode::UNAUTHORIZED
            | reqwest::StatusCode::FORBIDDEN
            | reqwest::StatusCode::NOT_FOUND
    ) {
        return HubManifest::Denied;
    }
    if !response.status().is_success() {
        return HubManifest::Unavailable;
    }
    let Some(digest) = response
        .headers()
        .get("docker-content-digest")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_owned())
    else {
        return HubManifest::Unavailable;
    };
    if digest.len() == 71 && digest.starts_with("sha256:") {
        HubManifest::Digest(digest)
    } else {
        HubManifest::Unavailable
    }
}

fn local_image_id(reference: &str) -> Option<String> {
    let mut command = docker_command();
    command.args([
        "image",
        "inspect",
        "--format",
        "{{.Id}}",
        "--",
        reference,
    ]);
    let output = run_local_docker(
        command,
        None,
        Duration::from_secs(60),
        "读取加速镜像标识",
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!id.is_empty()).then_some(id)
}

fn remove_local_image(reference: &str) {
    let mut command = docker_command();
    command.args(["rmi", reference]);
    let _ = run_local_docker(
        command,
        None,
        Duration::from_secs(60),
        "清理加速镜像引用",
    );
}

fn mirror_manifest_digest(
    address: &str,
    repository: &str,
    tag: &str,
) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("创建请求失败：{error}"))?;
    let url = format!("https://{address}/v2/{repository}/manifests/{tag}");
    let accept = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json";
    let mut response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, accept)
        .send()
        .map_err(|error| format!("请求失败：{error}"))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        let challenge = response
            .headers()
            .get("www-authenticate")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| String::from("未返回认证信息"))?;
        let token = registry_bearer_token(&client, challenge, repository)
            .ok_or_else(|| String::from("获取认证令牌失败"))?;
        response = client
            .get(&url)
            .header(reqwest::header::ACCEPT, accept)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .map_err(|error| format!("认证后请求失败：{error}"))?;
    }
    if !response.status().is_success() {
        return Err(format!("清单请求返回 {}", response.status()));
    }
    let bytes = response
        .bytes()
        .map_err(|error| format!("读取清单失败：{error}"))?;
    Ok(format!("sha256:{}", sha256_hex(&bytes)))
}

fn registry_bearer_token(
    client: &reqwest::blocking::Client,
    challenge: &str,
    repository: &str,
) -> Option<String> {
    let (realm, service, scope) = bearer_challenge_parts(challenge)?;
    let scope = scope.unwrap_or_else(|| format!("repository:{repository}:pull"));
    let mut request = client
        .get(realm)
        .query(&[("scope", scope.as_str())]);
    if let Some(service) = service.as_deref() {
        request = request.query(&[("service", service)]);
    }
    let value: Value = serde_json::from_slice(
        &request
            .send()
            .ok()?
            .error_for_status()
            .ok()?
            .bytes()
            .ok()?,
    )
    .ok()?;
    value
        .get("token")
        .or_else(|| value.get("access_token"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn bearer_challenge_parts(
    challenge: &str,
) -> Option<(String, Option<String>, Option<String>)> {
    let challenge = challenge
        .strip_prefix("Bearer ")
        .or_else(|| challenge.strip_prefix("bearer "))?;
    let mut realm = None;
    let mut service = None;
    let mut scope = None;
    for part in challenge.split(',') {
        let (key, value) = part.trim().split_once('=')?;
        let value = value.trim().trim_matches('"');
        match key.trim().to_ascii_lowercase().as_str() {
            "realm" => realm = Some(value.to_owned()),
            "service" => service = Some(value.to_owned()),
            "scope" => scope = Some(value.to_owned()),
            _ => {}
        }
    }
    let realm = realm?;
    Some((realm, service, scope))
}

fn docker_hub_auth_configured() -> bool {
    let dir = std::env::var_os("DOCKER_CONFIG")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| dirs::home_dir().map(|home| home.join(".docker")));
    let Some(dir) = dir else {
        return false;
    };
    let Ok(bytes) = std::fs::read(dir.join("config.json")) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return true;
    };
    if config_has_docker_hub_auth(&value) {
        return true;
    }
    if let Some(store) = value
        .get("credsStore")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|store| !store.is_empty())
    {
        return credential_helper_has_docker_hub(store);
    }
    if let Some(helper) = value
        .get("credHelpers")
        .and_then(Value::as_object)
        .and_then(|entries| {
            entries
                .iter()
                .find(|(host, _)| host.to_ascii_lowercase().contains("docker.io"))
                .and_then(|(_, helper)| helper.as_str())
        })
    {
        return credential_helper_has_docker_hub(helper);
    }
    false
}

fn config_has_docker_hub_auth(value: &Value) -> bool {
    let Some(entries) = value.get("auths").and_then(Value::as_object) else {
        return false;
    };
    entries.iter().any(|(host, entry)| {
        host.to_ascii_lowercase().contains("docker.io")
            && (entry
                .get("auth")
                .and_then(Value::as_str)
                .is_some_and(|auth| !auth.trim().is_empty())
                || entry
                    .get("identitytoken")
                    .and_then(Value::as_str)
                    .is_some_and(|token| !token.trim().is_empty()))
    })
}

fn credential_helper_has_docker_hub(store: &str) -> bool {
    let helper = format!(
        "docker-credential-{store}{}",
        if cfg!(windows) { ".exe" } else { "" }
    );
    let mut command = Command::new(helper);
    if let Some(path) = docker_path_env() {
        command.env("PATH", path);
    }
    command.arg("list");
    let Ok(output) = process::run(
        command,
        Input::Bytes(vec![]),
        Options {
            timeout: Duration::from_secs(5),
            ..Default::default()
        },
    ) else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) else {
        return false;
    };
    value.as_object().is_some_and(|entries| {
        entries
            .keys()
            .any(|host| host.to_ascii_lowercase().contains("docker.io"))
    })
}

fn registry_connection_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    if [
        "unauthorized",
        "authentication required",
        "pull access denied",
        "denied:",
        "forbidden",
        "no matching manifest",
        "manifest invalid",
        "name unknown",
        "not found",
        "404",
        " 401",
        " 403",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return false;
    }
    [
        "failed to do request",
        "dial tcp",
        "no such host",
        "server misbehaving",
        "connectex",
        "actively refused",
        "connection refused",
        "connection reset",
        "connection attempt failed",
        "connection timed out",
        "network is unreachable",
        "getaddrinfo failed",
        "i/o timeout",
        "context deadline exceeded",
        "执行超时",
        "temporary failure in name resolution",
        "tls handshake timeout",
        "proxyconnect",
        "unexpected eof",
        " eof",
        "toomanyrequests",
        "too many requests",
        " 429",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn prepare_local_for_transfer(
    window: &tauri::Window,
    app: &AppHandle,
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<String>,
    references: &[String],
    logins: &HashMap<String, PullLogin>,
) -> Result<Vec<DeploymentImageTransferResult>, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err(String::from("请先选择远程目标主机"));
    }
    let user = user.trim();
    if user.is_empty() {
        return Err(String::from("请先填写目标主机登录用户"));
    }
    check_local_endpoint()?;
    let key = resolve_existing_key(key_path, host)?;
    let output = run_ssh_script(
        host,
        port,
        user,
        key.as_deref(),
        "docker info --format '{{.OSType}}/{{.Architecture}}'\n",
    )?;
    if !output.status.success() {
        return Err(String::from(
            "无法确认目标主机的 Docker 架构，请检查目标机 Docker 与 SSH 连接后重试",
        ));
    }
    let target_platform = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if target_platform.is_empty() {
        return Err(String::from(
            "目标主机未返回 Docker 架构，请检查目标机 Docker 后重试",
        ));
    }

    let total = references.len();
    let mut prepared = Vec::with_capacity(total);
    for reference in references {
        let existing = inspect_local_image(reference, None)?;
        let needs_pull = match existing.as_deref() {
            Some(platform) if platform_matches(platform, &target_platform) => false,
            // 摘要引用也可以按目标架构重新拉取；仓库中的摘要若不含目标架构，
            // Docker 会在拉取时明确提示找不到匹配版本。
            Some(_) => true,
            None => true,
        };
        prepared.push((reference, needs_pull));
    }
    let pull_registries: HashSet<String> = prepared
        .iter()
        .filter(|(_, needs_pull)| *needs_pull)
        .filter_map(|(reference, _)| login_for_reference(reference, logins))
        .map(|login| login.registry.clone())
        .collect();
    let config = if pull_registries.is_empty() {
        None
    } else {
        let config = TempDockerConfig::create_for_current_context()?;
        for registry in &pull_registries {
            let login = logins
                .get(registry)
                .ok_or_else(|| format!("缺少镜像仓库 {registry} 的登录凭据"))?;
            login_docker(
                &login.registry,
                &login.username,
                &login.password,
                &config.path,
            )?;
        }
        Some(config)
    };
    for (index, (reference, needs_pull)) in prepared.into_iter().enumerate() {
        let login = login_for_reference(reference, logins);
        let docker_config = if needs_pull && login.is_some() {
            config.as_ref()
        } else {
            None
        };
        emit_pull_progress(
            window,
            index * 100 / total,
            index + 1,
            total,
            reference,
            &format!(
                "正在本机准备 [{} / {}] {reference}，目标架构 {target_platform}",
                index + 1,
                total
            ),
        );
        if needs_pull {
            let command = local_pull_command(reference, &target_platform);
            let output = run_local_docker(
                command,
                docker_config,
                Duration::from_secs(1800),
                &format!("在本机拉取镜像 {reference}"),
            )?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let message = pull_failure(stderr.trim(), login.is_some());
                let message = redact_secret(
                    &message,
                    login
                        .map(|value| value.password.as_str())
                        .unwrap_or_default(),
                );
                return Err(format!("在本机拉取镜像 {reference} 失败：{message}"));
            }
            let platform = inspect_local_image(reference, None)?
                .ok_or_else(|| format!("本机拉取镜像 {reference} 后未能确认镜像状态"))?;
            if !platform_matches(&platform, &target_platform) {
                return Err(format!(
                    "本机镜像 {reference} 的平台 {platform} 与目标主机 {target_platform} 不一致，请检查镜像清单中的平台声明"
                ));
            }
        }
        emit_pull_progress(
            window,
            (index + 1) * 100 / total,
            index + 1,
            total,
            reference,
            &format!("本机镜像已准备 {reference}"),
        );
    }
    let mut bindings = HashMap::new();
    for reference in references.iter().filter(|reference| reference.contains("@sha256:")) {
        bindings.insert(reference.clone(), image_identity(reference)?);
    }
    let exported = export_images_blocking(window, app, references.to_vec())?;
    exported
        .into_iter()
        .map(|export| {
            let binding = bindings.get(&export.reference).cloned();
            if export.success {
                if let Some(binding) = binding.as_ref() {
                    if &image_identity(&export.reference)? != binding {
                        return Err("导出期间本机镜像发生变化，请重新准备镜像".to_string());
                    }
                }
            }
            Ok(DeploymentImageTransferResult { export, binding })
        })
        .collect()
}

fn local_pull_command(reference: &str, target_platform: &str) -> Command {
    let mut command = docker_command();
    let platform = canonical_platform(target_platform);
    command.args(["pull", "--platform", platform.as_str(), reference]);
    command
}

fn inspect_local_image(
    reference: &str,
    config: Option<&TempDockerConfig>,
) -> Result<Option<String>, String> {
    let mut command = docker_command();
    command.args([
        "image",
        "inspect",
        "--format",
        "{{.Os}}/{{.Architecture}}",
        "--",
        reference,
    ]);
    let output = run_local_docker(command, config, Duration::from_secs(60), "读取本机镜像状态")?;
    if output.status.success() {
        let platform = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if platform.is_empty() {
            return Err(format!(
                "无法确认本机镜像 {reference} 的平台，请检查 Docker 后重试"
            ));
        }
        return Ok(Some(platform));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.to_ascii_lowercase().contains("no such image") {
        Ok(None)
    } else {
        Err(format!(
            "无法读取本机镜像 {reference} 的状态：{}",
            pull_failure(stderr.trim(), false)
        ))
    }
}

fn run_local_docker(
    mut command: Command,
    config: Option<&TempDockerConfig>,
    timeout: Duration,
    action: &str,
) -> Result<std::process::Output, String> {
    if let Some(config) = config {
        command.env("DOCKER_CONFIG", &config.path);
    }
    process::run(
        command,
        Input::Bytes(vec![]),
        Options {
            timeout,
            ..Default::default()
        },
    )
    .map_err(|error| format!("{action}失败：{error}"))
}

fn local_docker_platform(config: Option<&TempDockerConfig>) -> Result<String, String> {
    let mut command = docker_command();
    command.args(["info", "--format", "{{.OSType}}/{{.Architecture}}"]);
    let output = run_local_docker(
        command,
        config,
        Duration::from_secs(30),
        "读取本机 Docker 平台",
    )?;
    if !output.status.success() {
        return Err(String::from(
            "无法确认本机 Docker 平台，请检查 Docker 状态后重试",
        ));
    }
    let platform = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if platform.is_empty() {
        return Err(String::from(
            "未读取到本机 Docker 平台，请检查 Docker 状态后重试",
        ));
    }
    Ok(platform)
}

fn inspect_local_pull(
    reference: &str,
    config: Option<&TempDockerConfig>,
) -> Result<String, String> {
    let mut command = docker_command();
    command.args([
        "image",
        "inspect",
        "--format",
        "{{.Os}}/{{.Architecture}}",
        "--",
        reference,
    ]);
    let output = run_local_docker(command, config, Duration::from_secs(60), "确认拉取结果")?;
    let platform = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || platform.is_empty() {
        return Err(format!(
            "镜像 {reference} 拉取后未能确认，请检查 Docker 状态和镜像地址"
        ));
    }
    Ok(platform)
}

fn pull_remote_blocking(
    window: &tauri::Window,
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<String>,
    references: &[String],
    logins: &HashMap<String, PullLogin>,
    on_confirmed: &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<Vec<DeploymentImagePullResult>, String> {
    if !logins.is_empty() {
        ensure_known_host(host, port)?;
    }
    let key = resolve_existing_key(key_path, host)?;
    let total = references.len();
    let mut results = Vec::with_capacity(total);
    for (index, reference) in references.iter().enumerate() {
        let login = login_for_reference(reference, logins);
        let password_b64 = match login {
            Some(login) => Some(
                base64_encode(&login.password)
                    .map_err(|_| String::from("无法准备镜像仓库登录凭据，请检查系统组件后重试"))?,
            ),
            None => None,
        };
        emit_pull_progress(
            window,
            index * 100 / total,
            index + 1,
            total,
            reference,
            &format!("正在目标主机拉取 [{} / {}] {reference}", index + 1, total),
        );
        let script = remote_pull_script(reference, login, password_b64.as_deref());
        let output = if login.is_some() {
            run_ssh_script_strict_with_timeout(
                host,
                port,
                user,
                key.as_deref(),
                &script,
                Duration::from_secs(1800),
            )
        } else {
            run_ssh_script_with_timeout(
                host,
                port,
                user,
                key.as_deref(),
                &script,
                Duration::from_secs(1800),
            )
        }
        .map_err(|error| format!("在目标主机拉取镜像 {reference} 失败：{error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let message = if stderr
                .to_ascii_lowercase()
                .contains("host key verification failed")
            {
                String::from("目标主机 SSH 指纹校验失败，请先确认主机指纹后再发送镜像仓库凭据")
            } else {
                redact_pull_error(
                    &pull_failure(stderr.trim(), login.is_some()),
                    login,
                    password_b64.as_deref(),
                )
            };
            return Err(format!("在目标主机拉取镜像 {reference} 失败：{message}"));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let target_platform = prefixed_value(&stdout, "TARGET_PLATFORM=")
            .ok_or_else(|| format!("目标主机未返回镜像 {reference} 的平台信息，请重新比对"))?;
        let image_platform = prefixed_value(&stdout, "IMAGE_PLATFORM=")
            .ok_or_else(|| format!("目标主机未返回镜像 {reference} 的平台，请重新比对"))?;
        if !platform_matches(&image_platform, &target_platform) {
            return Err(format!(
                "目标主机镜像 {reference} 的平台 {image_platform} 与 Docker {target_platform} 不一致，请使用对应架构的镜像"
            ));
        }
        if !stdout.lines().any(|line| line.trim() == "PULL_OK") {
            return Err(format!(
                "目标主机未返回镜像 {reference} 的就绪结果，请检查 Docker 状态和镜像地址"
            ));
        }
        on_confirmed(reference)?;
        emit_pull_progress(
            window,
            (index + 1) * 100 / total,
            index + 1,
            total,
            reference,
            &format!("已拉取 {reference}"),
        );
        results.push(DeploymentImagePullResult {
            reference: reference.clone(),
            message: String::from("已在目标主机拉取并确认"),
        });
    }
    Ok(results)
}

fn remote_pull_script(
    reference: &str,
    login: Option<&PullLogin>,
    password_b64: Option<&str>,
) -> String {
    let mut script = String::from("set -e\nset -o pipefail\n");
    if let Some(login) = login {
        script.push_str(
            "SOURCE=${DOCKER_CONFIG:-$HOME/.docker}\nCURRENT_CONTEXT=$(docker context show)\numask 077\nDOCKER_CONFIG=$(mktemp -d)\nexport DOCKER_CONFIG\ntrap 'rm -rf \"$DOCKER_CONFIG\"' EXIT\nif [ -d \"$SOURCE/contexts\" ]; then cp -a \"$SOURCE/contexts\" \"$DOCKER_CONFIG/\"; fi\nif [ -d \"$SOURCE/certs.d\" ]; then cp -a \"$SOURCE/certs.d\" \"$DOCKER_CONFIG/\"; fi\nif [ -n \"$CURRENT_CONTEXT\" ]; then export DOCKER_CONTEXT=\"$CURRENT_CONTEXT\"; fi\n",
        );
        script.push_str(&format!(
            "PASSWORD_B64={}\nprintf '%s' \"$PASSWORD_B64\" | base64 -d | docker login {} -u {} --password-stdin >/dev/null\n",
            shell_quote(password_b64.unwrap_or_default()),
            shell_quote(&login.registry),
            shell_quote(&login.username)
        ));
    }
    script.push_str(&format!(
        "target_platform=$(docker info --format '{{{{.OSType}}}}/{{{{.Architecture}}}}')\ntarget_os=${{target_platform%%/*}}\ntarget_arch=${{target_platform#*/}}\ncase \"$target_arch\" in x86_64|amd64) target_arch=amd64;; aarch64|arm64) target_arch=arm64;; esac\ntarget_platform=\"$target_os/$target_arch\"\ndocker pull --platform \"$target_platform\" {}\nimage_platform=$(docker image inspect --format '{{{{.Os}}}}/{{{{.Architecture}}}}' -- {})\nprintf 'TARGET_PLATFORM=%s\\n' \"$target_platform\"\nprintf 'IMAGE_PLATFORM=%s\\n' \"$image_platform\"\necho PULL_OK\n",
        shell_quote(reference),
        shell_quote(reference)
    ));
    script
}

fn prefixed_value(output: &str, prefix: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn redact_pull_error(
    message: &str,
    login: Option<&PullLogin>,
    password_b64: Option<&str>,
) -> String {
    let message = redact_secret(
        message,
        login
            .map(|value| value.password.as_str())
            .unwrap_or_default(),
    );
    match password_b64.filter(|value| !value.is_empty()) {
        Some(encoded) => message.replace(encoded, "[已隐藏]"),
        None => message,
    }
}

fn pull_failure(stderr: &str, has_login: bool) -> String {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("error getting credentials")
        && (lower.contains("executable file not found") || lower.contains("not found in $path"))
    {
        return String::from(
            "Docker 无法读取已保存的登录凭据，请重启应用后重试；仍失败时请选择对应镜像源并填写凭据",
        );
    }
    if !has_login
        && (lower.contains("unauthorized")
            || lower.contains("pull access denied")
            || lower.contains("authentication required")
            || lower.contains("denied: requested access"))
    {
        return String::from("镜像仓库需要登录，请选择对应镜像源并填写凭据后重试");
    }
    if lower.contains("no matching manifest")
        || (lower.contains("image platform") && lower.contains("does not match"))
    {
        return String::from("镜像没有目标架构的可用版本，请更换交付包或选择匹配架构的镜像");
    }
    if lower.contains("manifest unknown") || lower.contains("not found: manifest") {
        return String::from(
            "镜像仓库中没有该摘要对应的镜像，请确认摘要与仓库地址是否匹配",
        );
    }
    if registry_connection_failure(stderr) {
        log::warn!(target: "deployment", "镜像仓库连接失败：{stderr}");
        return String::from("无法连接镜像仓库，请检查网络、代理与 Docker 镜像加速配置后重试");
    }
    docker_failure(stderr)
}

fn platform_matches(image: &str, target: &str) -> bool {
    let Some((image_os, image_arch)) = image.trim().split_once('/') else {
        return false;
    };
    let Some((target_os, target_arch)) = target.trim().split_once('/') else {
        return false;
    };
    !image_os.is_empty()
        && !target_os.is_empty()
        && image_os == target_os
        && architecture(image_arch) == architecture(target_arch)
}

fn emit_pull_progress(
    window: &tauri::Window,
    percent: usize,
    current: usize,
    total: usize,
    reference: &str,
    message: &str,
) {
    let _ = window.emit(
        "image-library-progress",
        ImageLibraryProgress {
            phase: String::from("pull"),
            current,
            total,
            reference: reference.to_owned(),
            percent: percent.min(100),
            message: message.to_owned(),
            finished: current == total && percent >= 100,
        },
    );
}

fn validate_selection(references: &[String], declared: &[String]) -> Result<(), String> {
    if references.is_empty()
        || references
            .iter()
            .any(|reference| !valid_image_reference(reference) || !declared.contains(reference))
    {
        return Err(String::from("所选镜像不在部署清单中，请重新比对后重试"));
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

fn canonical_platform(value: &str) -> String {
    let trimmed = value.trim();
    match trimmed.split_once('/') {
        Some((os, arch)) if !os.is_empty() && !arch.is_empty() => {
            format!("{os}/{}", architecture(arch))
        }
        _ => trimmed.to_string(),
    }
}

fn validate_platform(image: &Value, target: &str, reference: &str) -> Result<(), String> {
    let (target_os, target_arch) = target
        .split_once('/')
        .filter(|(os, arch)| !os.is_empty() && !arch.is_empty())
        .ok_or_else(|| String::from("无法确认目标主机架构，请重新检测环境"))?;
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
    use crate::commands::registry_sources::RegistrySource;
    use serde_json::json;

    fn access(address: &str, username: &str, password: Option<&str>) -> RegistrySourceAccess {
        RegistrySourceAccess {
            source: RegistrySource {
                address: address.into(),
                username: username.into(),
                ..Default::default()
            },
            password: password.map(str::to_owned),
        }
    }

    fn reference_registries(references: &[&str]) -> HashSet<String> {
        references
            .iter()
            .map(|reference| normalize_registry_host(&registry_for_reference(reference)))
            .collect()
    }

    fn login_map(
        access: RegistrySourceAccess,
        password: Option<&str>,
        references: &[&str],
    ) -> Result<HashMap<String, PullLogin>, String> {
        let mut logins = HashMap::new();
        add_pull_login(
            &mut logins,
            access,
            password.map(str::to_owned),
            &reference_registries(references),
        )?;
        Ok(logins)
    }

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

    #[test]
    fn pull_accepts_only_unique_references_from_the_bundle() {
        let reference = String::from("demo/api:1.0");
        assert_eq!(
            dedupe_references(vec![
                reference.clone(),
                format!(" {reference} "),
                String::from("other/api:1.0"),
            ]),
            vec![reference.clone(), String::from("other/api:1.0")]
        );
        let declared = vec![reference];
        assert!(validate_selection(&[String::from("demo/api:2.0")], &declared).is_err());
    }

    #[test]
    fn pull_login_requires_credentials_for_the_declared_registry() {
        let logins = login_map(
            access("registry.example.com", "reader", Some("secret")),
            None,
            &["registry.example.com/demo/api:1.0"],
        )
        .unwrap();
        let login = logins.values().next().unwrap();
        assert_eq!(login.registry, "registry.example.com");
        assert_eq!(login.username, "reader");
        assert_eq!(login.password, "secret");

        assert!(login_map(
            access("registry.example.com", "reader", Some("secret")),
            None,
            &["other.example.com/demo/api:1.0"],
        )
        .is_err());
    }

    #[test]
    fn pull_login_allows_anonymous_access_but_rejects_unused_credentials() {
        assert!(login_map(
            access("registry.example.com", "", None),
            None,
            &["registry.example.com/demo/api:1.0"],
        )
        .unwrap()
        .is_empty());
        assert!(login_map(
            access("registry.example.com", "", None),
            Some("secret"),
            &["registry.example.com/demo/api:1.0"],
        )
        .is_err());
    }

    #[test]
    fn docker_hub_aliases_share_the_same_login_target() {
        assert_eq!(normalize_registry_host("registry-1.docker.io"), "docker.io");
        let logins = login_map(
            access("docker.io", "reader", Some("secret")),
            None,
            &["bitnamilegacy/kafka:4.0"],
        )
        .unwrap();
        let login = logins.values().next().unwrap();
        assert_eq!(login.registry, "docker.io");
    }

    #[test]
    fn mixed_registries_keep_one_login_per_registry() {
        let mut logins = HashMap::new();
        add_pull_login(
            &mut logins,
            access("docker.io", "reader", Some("docker-secret")),
            None,
            &reference_registries(&["redis:7.2.8", "registry.example.com/demo/api:1.0"]),
        )
        .unwrap();
        add_pull_login(
            &mut logins,
            access("registry.example.com", "reader", Some("private-secret")),
            None,
            &reference_registries(&["redis:7.2.8", "registry.example.com/demo/api:1.0"]),
        )
        .unwrap();
        assert_eq!(logins.len(), 2);
        assert_eq!(
            login_for_reference("redis:7.2.8", &logins).map(|login| login.password.as_str()),
            Some("docker-secret")
        );
        assert_eq!(
            login_for_reference("registry.example.com/demo/api:1.0", &logins)
                .map(|login| login.password.as_str()),
            Some("private-secret")
        );
    }

    #[test]
    fn remote_pull_logs_in_before_pulling_the_exact_reference() {
        let login = PullLogin {
            registry: String::from("registry.example.com"),
            username: String::from("reader"),
            password: String::from("secret"),
        };
        let script = remote_pull_script("demo/api:1.0", Some(&login), Some("c2VjcmV0"));
        assert!(script.contains("docker login 'registry.example.com'"));
        assert!(script.contains("docker pull --platform \"$target_platform\" 'demo/api:1.0'"));
        assert!(script.contains("echo PULL_OK"));
        assert!(script.contains("trap 'rm -rf \"$DOCKER_CONFIG\"' EXIT"));
        assert!(script.contains("docker context show"));
        assert!(script.contains("cp -a \"$SOURCE/contexts\""));
        assert!(script.contains("base64 -d"));
        assert!(!script.contains("secret"));
        assert!(script.contains("TARGET_PLATFORM="));
        assert!(script.contains("IMAGE_PLATFORM="));
    }

    #[test]
    fn image_and_docker_platforms_accept_known_architecture_aliases() {
        assert!(platform_matches("linux/amd64", "linux/x86_64"));
        assert!(platform_matches("linux/arm64", "linux/aarch64"));
        assert!(!platform_matches("linux/arm64", "linux/amd64"));
        assert!(!platform_matches("windows/amd64", "linux/amd64"));
        assert!(!platform_matches("", "linux/amd64"));
        assert!(!platform_matches("linux", "linux/amd64"));
    }

    #[test]
    fn remote_platform_output_is_parsed_and_secrets_are_redacted() {
        let output = "TARGET_PLATFORM=linux/amd64\nIMAGE_PLATFORM=linux/arm64\nPULL_OK\n";
        assert_eq!(
            prefixed_value(output, "TARGET_PLATFORM=").as_deref(),
            Some("linux/amd64")
        );
        assert_eq!(
            prefixed_value(output, "IMAGE_PLATFORM=").as_deref(),
            Some("linux/arm64")
        );
        let login = PullLogin {
            registry: String::from("registry.example.com"),
            username: String::from("reader"),
            password: String::from("secret"),
        };
        let message = redact_pull_error("failed secret c2VjcmV0", Some(&login), Some("c2VjcmV0"));
        assert!(!message.contains("secret"));
        assert!(!message.contains("c2VjcmV0"));
        assert!(message.contains("[已隐藏]"));
    }

    #[test]
    fn relay_pull_requests_the_target_platform_explicitly() {
        let command = local_pull_command("demo/api:1.0", "linux/aarch64");
        let debug = format!("{command:?}");
        assert!(debug.contains("--platform"));
        assert!(debug.contains("linux/arm64"));
        assert!(debug.contains("demo/api:1.0"));
        assert_eq!(canonical_platform("linux/x86_64"), "linux/amd64");
    }

    #[test]
    fn pull_failures_explain_credential_helper_and_missing_login() {
        assert!(pull_failure(
            "error getting credentials - err: exec: \"docker-credential-osxkeychain\": executable file not found in $PATH",
            false
        )
        .contains("登录凭据"));
        assert!(pull_failure("unauthorized: authentication required", false).contains("需要登录"));
        assert_eq!(
            pull_failure("unauthorized: authentication required", true),
            docker_failure("unauthorized: authentication required")
        );
        assert!(pull_failure(
            "no matching manifest for linux/arm64/v8 in the manifest list entries",
            false
        )
        .contains("目标架构"));
        assert!(pull_failure(
            "manifest unknown: manifest unknown",
            false
        )
        .contains("该摘要对应的镜像"));
    }

    #[test]
    fn docker_hub_mirror_repository_keeps_org_paths_and_fills_library_namespace() {
        assert_eq!(
            docker_hub_mirror_repository("bitnamilegacy/kafka:4.0", "library"),
            Some(("bitnamilegacy/kafka".to_string(), "4.0".to_string()))
        );
        assert_eq!(
            docker_hub_mirror_repository("nginx:1.27", "library"),
            Some(("library/nginx".to_string(), "1.27".to_string()))
        );
        assert_eq!(
            docker_hub_mirror_repository("docker.io/library/redis:7", ""),
            Some(("library/redis".to_string(), "7".to_string()))
        );
        assert_eq!(
            docker_hub_mirror_repository("index.docker.io/library/redis:7", "library"),
            Some(("library/redis".to_string(), "7".to_string()))
        );
        assert_eq!(
            docker_hub_mirror_repository("Docker.IO/library/redis:7", "library"),
            Some(("library/redis".to_string(), "7".to_string()))
        );
        assert!(docker_hub_mirror_repository(
            "demo/api@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "library"
        )
        .is_none());
    }

    #[test]
    fn docker_hub_tag_fallback_only_accepts_tagged_hub_images() {
        assert!(is_docker_hub_tag("bitnamilegacy/kafka:4.0"));
        assert!(is_docker_hub_tag("docker.io/library/redis:7"));
        assert!(!is_docker_hub_tag(
            "bitnamilegacy/kafka@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(!is_docker_hub_tag("registry.example.com/demo/api:1.0"));
    }

    #[test]
    fn registry_connection_errors_exclude_login_and_missing_manifest() {
        assert!(registry_connection_failure(
            "Error response from daemon: failed to resolve reference \"docker.io/bitnamilegacy/kafka:4.0\": failed to do request: Head \"https://registry-1.docker.io/v2/\": dial tcp: connectex: A connection attempt failed"
        ));
        assert!(registry_connection_failure(
            "dial tcp 127.0.0.1:8888: connectex: No connection could be made because the target machine actively refused it"
        ));
        assert!(registry_connection_failure(
            "Get https://registry-1.docker.io/v2/: temporary failure in name resolution"
        ));
        assert!(registry_connection_failure(
            "failed to do request: context deadline exceeded"
        ));
        assert!(registry_connection_failure(
            "read tcp: getaddrinfo failed"
        ));
        assert!(registry_connection_failure(
            "connection timed out"
        ));
        assert!(registry_connection_failure(
            "network is unreachable"
        ));
        assert!(registry_connection_failure(
            "unexpected EOF"
        ));
        assert!(!registry_connection_failure(
            "failed to resolve reference \"docker.io/demo/api:1.0\": unauthorized: authentication required"
        ));
        assert!(!registry_connection_failure(
            "failed to resolve reference \"docker.io/demo/api:1.0\": docker.io/demo/api:1.0: not found"
        ));
        assert!(!registry_connection_failure(
            "failed to resolve reference \"docker.io/demo/api:1.0\": no matching manifest for linux/amd64 in the manifest list entries"
        ));
        assert!(!registry_connection_failure(
            "failed to resolve reference \"docker.io/demo/api:1.0\": 404 Not Found"
        ));
        assert!(!registry_connection_failure(
            "failed to resolve reference \"docker.io/demo/api:1.0\": manifest invalid"
        ));
    }

    #[test]
    fn docker_hub_repository_paths_are_normalized_for_manifest_checks() {
        assert_eq!(
            docker_hub_repository_and_tag("bitnamilegacy/kafka:4.0"),
            Some(("bitnamilegacy/kafka".to_string(), "4.0".to_string()))
        );
        assert_eq!(
            docker_hub_repository_and_tag("nginx"),
            Some(("library/nginx".to_string(), "latest".to_string()))
        );
        assert_eq!(
            docker_hub_repository_and_tag("docker.io/library/redis:7"),
            Some(("library/redis".to_string(), "7".to_string()))
        );
        assert!(docker_hub_repository_and_tag("demo/api@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_none());
    }

    #[test]
    fn registry_bearer_challenge_is_parsed_for_accelerator_tokens() {
        let parts = bearer_challenge_parts(
            "Bearer realm=\"https://auth.example.com/token\",service=\"registry.example.com\",scope=\"repository:demo/api:pull\"",
        )
        .unwrap();
        assert_eq!(parts.0, "https://auth.example.com/token");
        assert_eq!(parts.1.as_deref(), Some("registry.example.com"));
        assert_eq!(parts.2.as_deref(), Some("repository:demo/api:pull"));
        assert!(bearer_challenge_parts("Basic realm=\"example\"").is_none());
    }

    #[test]
    fn docker_hub_auth_config_disables_automatic_fallback() {
        assert!(config_has_docker_hub_auth(&json!({
            "auths": {"https://index.docker.io/v1/": {"auth": "dXNlcjpwYXNz"}}
        })));
        assert!(!config_has_docker_hub_auth(&json!({
            "auths": {"https://index.docker.io/v1/": {}}
        })));
        assert!(!config_has_docker_hub_auth(&json!({
            "auths": {"registry.example.com": {}}
        })));
        assert!(!config_has_docker_hub_auth(&json!({})));
    }
}
