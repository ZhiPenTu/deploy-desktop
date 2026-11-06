use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::core::process;

use super::bundle::{ArtifactKind, ArtifactRoot, Bundle};
use super::docker_setup::docker_command;
use super::deployment_image_bindings::{local_bindings, remote_bindings, DeploymentImageBinding};
use super::remote_ssh::{resolve_existing_key, run_ssh_script, shell_quote};
use super::remote_update::{
    compose_service_views, match_tarball, normalize_image_ref, resolve_bundle_file,
    scan_image_files, ComposeServiceView,
};

const COMPOSE_LIMIT: usize = 1024 * 1024;
const CONTAINER_FORMAT: &str = "{{.Name}}\t{{.Config.Image}}\t{{.State.Status}}\t{{index .Config.Labels \"com.docker.compose.service\"}}\t{{index .Config.Labels \"com.docker.compose.project.working_dir\"}}\t{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentServiceReview {
    pub name: String,
    pub image: String,
    pub current_image: String,
    pub container_name: String,
    pub container_state: String,
    pub container_health: String,
    pub ports: Vec<String>,
    pub depends_on: Vec<String>,
    pub profiles: Vec<String>,
    /// added / changed / unchanged / removed; changes describe Compose declarations.
    pub change: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentImageReview {
    pub image_ref: String,
    pub services: Vec<String>,
    pub docker_present: bool,
    /// ready / missing / platform_mismatch / check_failed / unavailable / invalid.
    /// A tarball alone never makes an image ready.
    pub status: String,
    pub platform: String,
    pub target_platform: String,
    pub detail: String,
    pub pullable: bool,
    pub local_file: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentArtifactReview {
    pub key: String,
    pub label: String,
    pub source: String,
    pub root: String,
    pub path: String,
    pub target: String,
    pub kind: String,
    /// present / missing / incomplete / invalid / unconfirmed.
    pub status: String,
    pub expected_bytes: u64,
    pub actual_bytes: u64,
    pub expected_files: usize,
    pub actual_files: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentReview {
    pub compose_file: String,
    pub target_compose_file: String,
    // Read-only source snapshots, never substituted for the execution configuration.
    pub current_compose: String,
    pub desired_compose: String,
    pub install_dir: String,
    pub has_deployment: bool,
    pub target_checked: bool,
    pub image_check: String,
    pub docker_running: bool,
    pub compose_available: bool,
    pub compose_changed: bool,
    pub services: Vec<DeploymentServiceReview>,
    pub images: Vec<DeploymentImageReview>,
    pub target_platform: String,
    pub artifacts: Vec<DeploymentArtifactReview>,
    pub artifact_issues: Vec<String>,
    pub missing_images: Vec<String>,
    pub unready_images: Vec<String>,
    pub issues: Vec<String>,
    pub ready: bool,
    pub can_first_deploy: bool,
    pub can_incremental_update: bool,
}

/// Read-only comparison. No images are pulled, no files are uploaded and no containers are changed.
#[tauri::command]
pub async fn inspect_deployment_review(
    bundle_dir: String,
    install_dir: String,
    image_dir: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<DeploymentReview, String> {
    let review = tauri::async_runtime::spawn_blocking(move || {
        review_blocking(
            bundle_dir,
            install_dir,
            image_dir,
            host,
            port,
            user,
            key_path,
        )
    })
    .await
    .map_err(|_| "核对任务未能完成，请重试".to_string())
    .and_then(|inner| inner);
    match &review {
        Ok(value) => log::info!(
            target: "review",
            "部署核对完成：就绪={} 可首次部署={} 可增量更新={} 待处理 {} 项",
            value.ready,
            value.can_first_deploy,
            value.can_incremental_update,
            value.issues.len()
        ),
        Err(error) => log::error!(target: "review", "部署核对失败：{error}"),
    }
    review
}

fn review_blocking(
    bundle_dir: String,
    install_dir: String,
    image_dir: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<DeploymentReview, String> {
    let root = fs::canonicalize(bundle_dir.trim())
        .map_err(|_| "无法访问交付包目录，请重新选择并解压制品".to_string())?;
    let bundle = crate::commands::bundle::Bundle::load(&root)?;
    bundle.ensure_compatible()?;
    // 声明的变体清单才是目标环境实际运行的清单，比对与执行必须保持一致。
    let compose_path = bundle.effective_compose().to_string();
    if bundle.artifact_for_source(&compose_path).is_none() {
        return Err("交付包未声明 Docker Compose 清单，请重新选择制品".to_string());
    }
    let path = resolve_bundle_file(&root, &compose_path)?;
    let desired = read_compose(&path)?;
    let services = parse_services(&desired)?;
    if services.is_empty() {
        return Err("交付包的 Docker Compose 清单没有服务，请重新选择制品".to_string());
    }

    let install = install_dir.trim().trim_end_matches('/');
    if install.is_empty() || install.contains(['\n', '\r', '\0']) {
        return Err("请填写有效的目标部署目录".to_string());
    }
    let images = unique_images(&services);
    let remote = host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty());
    let artifacts = artifact_probes(&bundle, install, remote.is_some());
    let bindings;
    let target = if let Some(host) = remote {
        let user = user
            .as_deref()
            .map(str::trim)
            .filter(|user| !user.is_empty())
            .ok_or_else(|| "请选择具有 SSH 登录账号的主机".to_string())?;
        let key = resolve_existing_key(key_path, host)?;
        bindings = remote_bindings(host, port.unwrap_or(22), user, key.as_deref(), install)?;
        let output = run_ssh_script(
            host,
            port.unwrap_or(22),
            user,
            key.as_deref(),
            &target_script_with_bindings(install, &images, &artifacts, &bindings),
        )
        .map_err(|_| "无法连接目标主机，请检查主机连接和 SSH 凭证后重试".to_string())?;
        if !output.status.success() {
            return Err("无法完成目标主机核对，请检查 SSH 连接与部署目录的读取权限".to_string());
        }
        let target = parse_target_output(&String::from_utf8_lossy(&output.stdout))?;
        if !target.artifacts_checked {
            return Err(
                "目标主机未返回完整的制品文件核对结果，请检查连接后重新核对".to_string(),
            );
        }
        target
    } else {
        bindings = local_bindings(Path::new(install))?;
        inspect_local_target(Path::new(install), &images, &artifacts, &bindings)?
    };
    let mut review = compare_with_artifacts(
        &compose_path,
        install,
        &desired,
        &services,
        &images,
        target,
        &artifacts,
    )?;
    for image in &mut review.images {
        if image.status == "ready" && bindings.iter().any(|binding| binding.reference == image.image_ref) {
            image.detail = "目标 Docker 中的离线镜像已按源端镜像内容核对，可继续部署".to_string();
        }
    }
    review.image_check = if review.target_checked && review.docker_running {
        if remote.is_some() {
            "remote"
        } else {
            "local"
        }
    } else {
        "unavailable"
    }
    .to_string();

    // Package discovery is only a preparation hint. Docker readiness above is always checked
    // with the complete requested image reference, including any sha256 digest.
    let media = image_dir
        .as_deref()
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.clone());
    let media = if media.join("images").is_dir() {
        media.join("images")
    } else {
        media
    };
    let files = scan_image_files(&media);
    let manifest = super::image_manager::load_image_manifest(&media);
    let declared = super::remote_update::declared_images(&root);
    for image in &mut review.images {
        let (package, _) = match_tarball(
            &normalize_image_ref(&image.image_ref),
            &files,
            &manifest,
            &declared,
        );
        image.local_file = package.map(|(name, _)| name).unwrap_or_default();
    }
    Ok(review)
}

fn read_compose(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path)
        .map_err(|_| "无法读取 Docker Compose 清单，请检查文件权限".to_string())?;
    if metadata.len() > COMPOSE_LIMIT as u64 {
        return Err("Docker Compose 清单超过 1 MB，无法完成核对".to_string());
    }
    fs::read_to_string(path)
        .map_err(|_| "无法读取 Docker Compose 清单，请确认文件使用 UTF-8 编码".to_string())
}

fn parse_services(content: &str) -> Result<Vec<ComposeServiceView>, String> {
    // Do not return parser excerpts: a YAML document may contain credentials.
    compose_service_views(content)
        .map_err(|_| "Docker Compose 清单格式有误，请检查清单后重试".to_string())
}

fn unique_images(services: &[ComposeServiceView]) -> Vec<String> {
    let mut images = Vec::new();
    for service in services {
        if !service.image.is_empty() && !images.contains(&service.image) {
            images.push(service.image.clone());
        }
    }
    images
}

#[derive(Debug, Clone)]
struct ArtifactExpectedFile {
    relative: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Clone)]
struct ArtifactProbe {
    key: String,
    label: String,
    source: String,
    root: String,
    path: String,
    kind: ArtifactKind,
    target: String,
    expected_bytes: u64,
    expected_files: usize,
    expected: Vec<ArtifactExpectedFile>,
}

fn posix_join(base: &str, child: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let child = child.trim().trim_start_matches('/');
    if child.is_empty() {
        base.to_string()
    } else if base.is_empty() {
        child.to_string()
    } else {
        format!("{base}/{child}")
    }
}

fn artifact_probes(bundle: &Bundle, install: &str, remote: bool) -> Vec<ArtifactProbe> {
    let deploy_rel = bundle.deploy_source_rel();
    let local_install = Path::new(install);
    let local_deploy = local_install.join(&deploy_rel);
    let remote_install = install.trim().trim_end_matches('/');
    let remote_deploy = posix_join(remote_install, &deploy_rel);
    bundle
        .descriptor
        .artifacts
        .iter()
        .map(|artifact| {
            let source = artifact.source.trim();
            let owned: Vec<_> = match artifact.kind {
                ArtifactKind::File => bundle
                    .descriptor
                    .files
                    .iter()
                    .filter(|file| file.path.trim() == source)
                    .collect(),
                ArtifactKind::Dir => {
                    let prefix = format!("{}/", source.trim_end_matches('/'));
                    bundle
                        .descriptor
                        .files
                        .iter()
                        .filter(|file| file.path.trim().starts_with(&prefix))
                        .collect()
                }
            };
            let (local_base, remote_base) = match artifact.root {
                ArtifactRoot::Install => (local_install.to_path_buf(), remote_install),
                ArtifactRoot::Deploy => (local_deploy.clone(), remote_deploy.as_str()),
            };
            let target = if remote {
                posix_join(remote_base, &artifact.path)
            } else {
                local_base
                    .join(artifact.path.trim())
                    .to_string_lossy()
                    .to_string()
            };
            let expected: Vec<ArtifactExpectedFile> = owned
                .iter()
                .map(|file| {
                    let relative = match artifact.kind {
                        ArtifactKind::File => String::new(),
                        ArtifactKind::Dir => file
                            .path
                            .trim()
                            .strip_prefix(source.trim_end_matches('/'))
                            .unwrap_or("")
                            .trim_start_matches('/')
                            .to_string(),
                    };
                    ArtifactExpectedFile {
                        relative,
                        size: file.size,
                        sha256: file.sha256.to_ascii_lowercase(),
                    }
                })
                .collect();
            ArtifactProbe {
                key: artifact.key.clone(),
                label: if artifact.label.trim().is_empty() {
                    artifact.key.clone()
                } else {
                    artifact.label.clone()
                },
                source: source.to_string(),
                root: artifact.root.as_str().to_string(),
                path: artifact.path.clone(),
                kind: artifact.kind,
                target,
                expected_bytes: owned.iter().map(|file| file.size).sum(),
                expected_files: owned.len(),
                expected,
            }
        })
        .collect()
}

fn inspect_local_target(
    install: &Path,
    images: &[String],
    artifacts: &[ArtifactProbe],
    bindings: &[DeploymentImageBinding],
) -> Result<TargetSnapshot, String> {
    check_local_endpoint()?;
    let mut target = local_compose_snapshot(install)?;
    let mut info_command = docker_command();
    info_command.arg("info");
    target.docker_running = run_local_docker(info_command, Duration::from_secs(15))
        .map(|output| output.status.success())
        .unwrap_or(false);
    let mut compose_command = docker_command();
    compose_command.args(["compose", "version"]);
    target.compose_available = run_local_docker(compose_command, Duration::from_secs(15))
        .map(|output| output.status.success())
        .unwrap_or(false);
    let target_platform = if target.docker_running {
        local_docker_platform().unwrap_or_default()
    } else {
        String::new()
    };
    target.target_platform = target_platform.clone();
    if target.docker_running {
        let mut ps_command = docker_command();
        ps_command.args(["ps", "-aq"]);
        let containers = run_local_docker(ps_command, Duration::from_secs(15))
            .map_err(|_| "无法读取本机容器状态，请检查 Docker 后重试".to_string())?;
        if !containers.status.success() {
            return Err("无法读取本机容器状态，请检查 Docker 后重试".to_string());
        }
        let ids = String::from_utf8_lossy(&containers.stdout);
        let ids: Vec<&str> = ids
            .lines()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .collect();
        if !ids.is_empty() {
            let mut command = docker_command();
            command
                .args(["inspect", "--format", CONTAINER_FORMAT, "--"])
                .args(&ids);
            let output = run_local_docker(command, Duration::from_secs(30))
                .map_err(|_| "无法核对本机容器状态，请重试".to_string())?;
            if !output.status.success() {
                return Err("无法核对本机容器状态，请重试".to_string());
            }
            target.containers = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| {
                    parse_container(line)
                        .ok_or_else(|| "容器状态核对结果不完整，请检查 Docker 后重试".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if target.containers.len() != ids.len() {
                return Err("容器状态核对结果不完整，请检查 Docker 后重试".to_string());
            }
        }
        target.containers_checked = true;
    }
    for (index, reference) in images.iter().enumerate() {
        let snapshot = if !target.docker_running || !valid_image_reference(reference) {
            ImageSnapshot {
                status: if target.docker_running {
                    "invalid".to_string()
                } else {
                    "unavailable".to_string()
                },
                platform: String::new(),
            }
        } else {
            let binding = bindings.iter().find(|binding| &binding.reference == reference);
            let inspected = binding
                .map(|binding| binding.image_id.as_str())
                .unwrap_or(reference);
            let mut command = docker_command();
            command.args([
                "image",
                "inspect",
                "--format",
                "{{.Id}}|{{.Os}}/{{.Architecture}}",
                "--",
                inspected,
            ]);
            match run_local_docker(command, Duration::from_secs(15)) {
                Ok(output) if output.status.success() => {
                    let text = String::from_utf8_lossy(&output.stdout);
                    match text.trim().split_once('|') {
                        Some((id, platform))
                            if valid_image_id(id)
                                && binding.map_or(true, |binding| {
                                    binding.image_id == id
                                        && platform_matches(platform, &binding.platform)
                                })
                                && platform_matches(platform, &target_platform) =>
                        {
                            ImageSnapshot {
                                status: "ready".to_string(),
                                platform: platform.to_string(),
                            }
                        }
                        Some((id, platform))
                            if valid_image_id(id)
                                && binding.map_or(true, |binding| {
                                    binding.image_id == id
                                        && platform_matches(platform, &binding.platform)
                                }) => ImageSnapshot {
                            status: "platform_mismatch".to_string(),
                            platform: platform.to_string(),
                        },
                        _ => ImageSnapshot {
                            status: "check_failed".to_string(),
                            platform: String::new(),
                        },
                    }
                }
                Ok(output)
                    if String::from_utf8_lossy(&output.stderr)
                        .to_ascii_lowercase()
                        .contains("no such image") =>
                {
                    ImageSnapshot {
                        status: "missing".to_string(),
                        platform: String::new(),
                    }
                }
                Ok(_) => ImageSnapshot {
                    status: "check_failed".to_string(),
                    platform: String::new(),
                },
                Err(_) => ImageSnapshot {
                    status: "check_failed".to_string(),
                    platform: String::new(),
                },
            }
        };
        target.image_states.insert(index, snapshot);
    }
    target.artifact_states = inspect_local_artifacts(artifacts);
    target.artifacts_checked = true;
    Ok(target)
}

fn inspect_local_artifacts(probes: &[ArtifactProbe]) -> HashMap<usize, ArtifactSnapshot> {
    probes
        .iter()
        .enumerate()
        .map(|(index, probe)| (index, local_artifact_snapshot(probe)))
        .collect()
}

fn local_artifact_snapshot(probe: &ArtifactProbe) -> ArtifactSnapshot {
    let path = Path::new(&probe.target);
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ArtifactSnapshot {
                status: "missing".to_string(),
                ..Default::default()
            };
        }
        Err(_) => {
            return ArtifactSnapshot {
                status: "unconfirmed".to_string(),
                ..Default::default()
            };
        }
    };
    if metadata.file_type().is_symlink() {
        return ArtifactSnapshot {
            status: "invalid".to_string(),
            ..Default::default()
        };
    }
    match probe.kind {
        ArtifactKind::File if metadata.is_file() => {
            local_file_snapshot(path, probe.expected.first())
        }
        ArtifactKind::Dir if metadata.is_dir() => match local_tree_unsafe(path) {
            Ok(true) => ArtifactSnapshot {
                status: "invalid".to_string(),
                ..Default::default()
            },
            Ok(false) => match directory_stats(path) {
                Ok((bytes, files)) => ArtifactSnapshot {
                    status: local_directory_status(path, &probe.expected),
                    actual_bytes: bytes,
                    actual_files: files,
                },
                Err(_) => ArtifactSnapshot {
                    status: "unconfirmed".to_string(),
                    ..Default::default()
                },
            },
            Err(_) => ArtifactSnapshot {
                status: "unconfirmed".to_string(),
                ..Default::default()
            },
        },
        _ => ArtifactSnapshot {
            status: "invalid".to_string(),
            ..Default::default()
        },
    }
}

fn local_file_snapshot(path: &Path, expected: Option<&ArtifactExpectedFile>) -> ArtifactSnapshot {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => {
            return ArtifactSnapshot {
                status: "unconfirmed".to_string(),
                ..Default::default()
            }
        }
    };
    let Some(expected) = expected else {
        return ArtifactSnapshot {
            status: "present".to_string(),
            actual_bytes: metadata.len(),
            actual_files: 1,
        };
    };
    if metadata.len() != expected.size {
        return ArtifactSnapshot {
            status: "incomplete".to_string(),
            actual_bytes: metadata.len(),
            actual_files: 1,
        };
    }
    match crate::commands::bundle::sha256_path(path) {
        Ok(digest) if digest == expected.sha256 => ArtifactSnapshot {
            status: "present".to_string(),
            actual_bytes: metadata.len(),
            actual_files: 1,
        },
        Ok(_) => ArtifactSnapshot {
            status: "incomplete".to_string(),
            actual_bytes: metadata.len(),
            actual_files: 1,
        },
        Err(_) => ArtifactSnapshot {
            status: "unconfirmed".to_string(),
            actual_bytes: metadata.len(),
            actual_files: 1,
        },
    }
}

fn local_directory_status(path: &Path, expected: &[ArtifactExpectedFile]) -> String {
    for file in expected {
        let target = if file.relative.is_empty() {
            path.to_path_buf()
        } else {
            path.join(&file.relative)
        };
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => return "invalid".to_string(),
            Ok(metadata) if !metadata.is_file() => return "incomplete".to_string(),
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return "incomplete".to_string()
            }
            Err(_) => return "unconfirmed".to_string(),
        };
        if metadata.len() != file.size {
            return "incomplete".to_string();
        }
        match crate::commands::bundle::sha256_path(&target) {
            Ok(digest) if digest == file.sha256 => {}
            Ok(_) => return "incomplete".to_string(),
            Err(_) => return "unconfirmed".to_string(),
        }
    }
    "present".to_string()
}

fn local_tree_unsafe(path: &Path) -> std::io::Result<bool> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Ok(true);
        }
        if file_type.is_dir() {
            if local_tree_unsafe(&entry.path())? {
                return Ok(true);
            }
        } else if !file_type.is_file() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn directory_stats(path: &Path) -> std::io::Result<(u64, usize)> {
    let mut bytes = 0;
    let mut files = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let (child_bytes, child_files) = directory_stats(&entry.path())?;
            bytes += child_bytes;
            files += child_files;
        } else if file_type.is_file() {
            bytes += entry.metadata()?.len();
            files += 1;
        }
    }
    Ok((bytes, files))
}

fn local_compose_snapshot(install: &Path) -> Result<TargetSnapshot, String> {
    let mut target = TargetSnapshot {
        checked: true,
        ..TargetSnapshot::default()
    };
    for name in crate::core::compose::SEARCH_CANDIDATES {
        let path = install.join(name);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {
                target.compose = read_compose(&path)?;
                target.compose_file = path.to_string_lossy().to_string();
                target.compose_working_dir = fs::canonicalize(path.parent().unwrap_or(install))
                    .map_err(|_| "无法访问本机部署目录，请检查目录权限".to_string())?
                    .to_string_lossy()
                    .to_string();
                break;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("无法读取本机部署目录，请检查目录权限后重试".to_string()),
        }
    }
    Ok(target)
}

pub(super) fn check_local_endpoint() -> Result<(), String> {
    let mut command = docker_command();
    command.args([
        "context",
        "inspect",
        "--format",
        "{{.Endpoints.docker.Host}}",
    ]);
    let context = run_local_docker(command, Duration::from_secs(15))
        .map_err(|_| "无法读取 Docker 连接，请确认已安装 Docker".to_string())?;
    if !context.status.success() {
        return Err("无法读取 Docker 连接，请检查本机 Docker 配置后重试".to_string());
    }
    let context = String::from_utf8_lossy(&context.stdout);
    let host_override = std::env::var("DOCKER_HOST").ok();
    let context_override = std::env::var("DOCKER_CONTEXT").ok();
    if !local_endpoint(
        &context,
        host_override.as_deref(),
        context_override.as_deref(),
    ) {
        return Err("本机部署需要连接本机 Docker，请先切换 Docker 连接后重新核对".to_string());
    }
    Ok(())
}

fn run_local_docker(command: Command, timeout: Duration) -> Result<std::process::Output, String> {
    process::run(
        command,
        process::Input::Bytes(vec![]),
        process::Options {
            timeout,
            ..Default::default()
        },
    )
}

fn local_docker_platform() -> Result<String, String> {
    let mut command = docker_command();
    command.args(["info", "--format", "{{.OSType}}/{{.Architecture}}"]);
    let output = run_local_docker(command, Duration::from_secs(15))?;
    if !output.status.success() {
        return Err("无法确认本机 Docker 平台，请检查 Docker 状态后重试".to_string());
    }
    let platform = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if platform.is_empty() {
        return Err("未读取到本机 Docker 平台，请检查 Docker 状态后重试".to_string());
    }
    Ok(platform)
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

fn local_endpoint(
    context: &str,
    host_override: Option<&str>,
    context_override: Option<&str>,
) -> bool {
    let endpoint = if context_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        context
    } else {
        host_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(context)
    }
    .trim();
    endpoint.starts_with("unix://") || endpoint.starts_with("npipe://")
}

fn valid_image_id(reference: &str) -> bool {
    reference
        .strip_prefix("sha256:")
        .map(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .unwrap_or(false)
}

pub(super) fn valid_image_reference(reference: &str) -> bool {
    if reference.is_empty()
        || reference.len() > 512
        || !reference.is_ascii()
        || reference.starts_with(['-', '/', '.'])
        || reference
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || reference.contains("://")
        || reference.contains(['$', '\\', '\'', '"', '`', ';', '|', '&', '(', ')', '{', '}'])
    {
        return false;
    }
    let name = if let Some((name, digest)) = reference.split_once('@') {
        if !valid_image_id(digest) || name.is_empty() || name.contains('@') {
            return false;
        }
        name
    } else {
        reference
    };
    let last_part = name.rsplit('/').next().unwrap_or_default();
    let repository = if let Some((_, tag)) = last_part.rsplit_once(':') {
        if tag.is_empty()
            || tag.len() > 128
            || tag.starts_with(['.', '-'])
            || !tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            return false;
        }
        &name[..name.len() - tag.len() - 1]
    } else {
        name
    };
    let segments: Vec<&str> = repository.split('/').collect();
    segments.iter().enumerate().all(|(index, segment)| {
        if segment.is_empty() {
            return false;
        }
        if index == 0
            && segments.len() > 1
            && (segment.contains('.') || segment.contains(':') || *segment == "localhost")
        {
            return segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b));
        }
        let bytes = segment.as_bytes();
        bytes
            .first()
            .map(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            .unwrap_or(false)
            && bytes
                .last()
                .map(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                .unwrap_or(false)
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(b))
            && !segment.contains("..")
    })
}

#[cfg(test)]
fn target_script(install: &str, images: &[String]) -> String {
    target_script_with_artifacts(install, images, &[])
}

#[cfg(test)]
fn target_script_with_artifacts(
    install: &str,
    images: &[String],
    artifacts: &[ArtifactProbe],
) -> String {
    target_script_with_bindings(install, images, artifacts, &[])
}

fn target_script_with_bindings(
    install: &str,
    images: &[String],
    artifacts: &[ArtifactProbe],
    bindings: &[DeploymentImageBinding],
) -> String {
    let mut script = format!(
        r#"set -euo pipefail
INSTALL={install}
normalize_arch() {{ case "$1" in x86_64|amd64) printf 'amd64';; aarch64|arm64) printf 'arm64';; *) printf '%s' "$1";; esac; }}
artifact_stat() {{ stat -c '%s' "$1" 2>/dev/null || stat -f '%z' "$1" 2>/dev/null || printf '0'; }}
artifact_parent_state() {{
  dir="$1"
  while [ -n "$dir" ] && [ "$dir" != "/" ]; do
    if [ -d "$dir" ]; then
      if [ -r "$dir" ] && [ -x "$dir" ]; then return 0; fi
      return 1
    fi
    dir=$(dirname "$dir")
  done
  if [ -x "/" ]; then return 0; fi
  return 1
}}
artifact_sha256() {{
  if command -v sha256sum >/dev/null 2>&1; then
    if hash_output=$(sha256sum "$1" 2>/dev/null); then printf '%s' "${{hash_output%% *}}"; fi
  elif command -v shasum >/dev/null 2>&1; then
    if hash_output=$(shasum -a 256 "$1" 2>/dev/null); then printf '%s' "${{hash_output%% *}}"; fi
  elif command -v openssl >/dev/null 2>&1; then
    if hash_output=$(openssl dgst -sha256 "$1" 2>/dev/null); then printf '%s' "${{hash_output##* }}"; fi
  else
    printf ''
  fi
}}
target_platform=$(docker info --format '{{{{.OSType}}}}/{{{{.Architecture}}}}' 2>/dev/null || true)
printf 'TARGET_PLATFORM=%s\n' "$target_platform"
{compose_location}
if [ -d "$INSTALL" ] && {{ [ ! -r "$INSTALL" ] || [ ! -x "$INSTALL" ]; }}; then exit 3; fi
if [ -d "$DEPLOY" ] && {{ [ ! -r "$DEPLOY" ] || [ ! -x "$DEPLOY" ]; }}; then exit 3; fi
printf 'TARGET_CHECKED=yes\nCOMPOSE_FILE=%s\n' "$COMPOSE_FILE"
if docker info >/dev/null 2>&1; then
  echo 'DOCKER_RUNNING=yes'
else
  echo 'DOCKER_RUNNING=no'
fi
if docker compose version >/dev/null 2>&1; then
  echo 'COMPOSE_AVAILABLE=yes'
else
  echo 'COMPOSE_AVAILABLE=no'
fi
if [ -n "$COMPOSE_FILE" ]; then
  printf 'COMPOSE_WORKING_DIR=%s\n' "$(cd "$(dirname "$COMPOSE_FILE")" && pwd -P)"
  [ -r "$COMPOSE_FILE" ] || exit 3
  size=$(wc -c < "$COMPOSE_FILE")
  [ "$size" -le {limit} ] || exit 4
  echo '__REVIEW_COMPOSE_BEGIN__'
  cat "$COMPOSE_FILE"
  printf '\n__REVIEW_COMPOSE_END__\n'
fi
echo '__REVIEW_CONTAINERS_BEGIN__'
if docker info >/dev/null 2>&1; then
  containers=$(docker ps -aq 2>/dev/null) || exit 5
  if [ -n "$containers" ]; then
    docker inspect --format {container_format} -- $containers 2>/dev/null || exit 5
  fi
fi
echo '__REVIEW_CONTAINERS_END__'
echo 'CONTAINERS_CHECKED=yes'
"#,
        install = shell_quote(install),
        compose_location = super::docker_ops::compose_location_script(),
        limit = COMPOSE_LIMIT,
        container_format = shell_quote(CONTAINER_FORMAT)
    );
    for (index, reference) in images.iter().enumerate() {
        if !valid_image_reference(reference) {
            script.push_str(&format!("echo 'IMAGE|{index}|unconfirmed'\n"));
            continue;
        }
        let binding = bindings.iter().find(|binding| &binding.reference == reference);
        let inspected = binding
            .map(|binding| binding.image_id.as_str())
            .unwrap_or(reference);
        let inspection = format!(
            "image_info=$(docker image inspect --format '{{{{.Id}}}}|{{{{.Os}}}}/{{{{.Architecture}}}}' -- {} 2>&1)",
            shell_quote(inspected)
        );
        let identity_check = binding.map(|binding| format!(
            "[ \"$image_id\" = {} ] && [ \"$image_os/$(normalize_arch \"$image_arch\")\" = {} ]",
            shell_quote(&binding.image_id),
            shell_quote(&canonical_platform(&binding.platform))
        )).unwrap_or_else(|| "true".to_owned());
        script.push_str(&format!(
            "if ! docker info >/dev/null 2>&1; then\n  echo 'IMAGE|{index}|unavailable||'\nelif {inspection}; then\n  image_id=${{image_info%%|*}}\n  image_platform=${{image_info#*|}}\n  target_os=${{target_platform%%/*}}\n  target_arch=${{target_platform#*/}}\n  image_os=${{image_platform%%/*}}\n  image_arch=${{image_platform#*/}}\n  if ! ( {identity_check} ); then\n    echo 'IMAGE|{index}|check_failed||'\n  elif [ \"$image_os\" = \"$target_os\" ] && [ \"$(normalize_arch \"$image_arch\")\" = \"$(normalize_arch \"$target_arch\")\" ]; then\n    printf 'IMAGE|{index}|ready|%s|%s\\n' \"$image_id\" \"$image_platform\"\n  else\n    printf 'IMAGE|{index}|platform_mismatch|%s|%s\\n' \"$image_id\" \"$image_platform\"\n  fi\nelse\n  case \"$image_info\" in *[Nn]o\\ such\\ image*) printf 'IMAGE|{index}|missing||\\n';; *) printf 'IMAGE|{index}|check_failed||\\n';; esac\nfi\n"
        ));
    }
    for (index, artifact) in artifacts.iter().enumerate() {
        script.push_str(&remote_artifact_probe_script(index, artifact));
    }
    script.push_str("echo 'ARTIFACTS_CHECKED=yes'\n");
    script
}

fn remote_artifact_probe_script(index: usize, probe: &ArtifactProbe) -> String {
    let path = shell_quote(&probe.target);
    match probe.kind {
        ArtifactKind::File => {
            let Some(expected) = probe.expected.first() else {
                return format!(
                    "ARTIFACT_PATH={path}\nif [ -L \"$ARTIFACT_PATH\" ]; then\n  printf 'ARTIFACT|{index}|invalid|0|0\\n'\nelif [ -f \"$ARTIFACT_PATH\" ]; then\n  printf 'ARTIFACT|{index}|present|%s|1\\n' \"$(artifact_stat \"$ARTIFACT_PATH\")\"\nelif [ -e \"$ARTIFACT_PATH\" ]; then\n  printf 'ARTIFACT|{index}|invalid|0|0\\n'\nelse\n  artifact_parent_state \"$(dirname \"$ARTIFACT_PATH\")\" || artifact_state=$?\n  if [ \"${{artifact_state:-0}}\" -eq 1 ]; then\n    printf 'ARTIFACT|{index}|unconfirmed|0|0\\n'\n  else\n    printf 'ARTIFACT|{index}|missing|0|0\\n'\n  fi\nfi\n"
                );
            };
            format!(
                "ARTIFACT_PATH={path}\nif [ -L \"$ARTIFACT_PATH\" ]; then\n  printf 'ARTIFACT|{index}|invalid|0|0\\n'\nelif [ -f \"$ARTIFACT_PATH\" ]; then\n  artifact_size=$(artifact_stat \"$ARTIFACT_PATH\")\n  artifact_digest=$(artifact_sha256 \"$ARTIFACT_PATH\")\n  if [ -z \"$artifact_digest\" ]; then\n    printf 'ARTIFACT|{index}|unconfirmed|%s|1\\n' \"$artifact_size\"\n  elif [ \"$artifact_size\" = {size} ] && [ \"$artifact_digest\" = {digest} ]; then\n    printf 'ARTIFACT|{index}|present|%s|1\\n' \"$artifact_size\"\n  else\n    printf 'ARTIFACT|{index}|incomplete|%s|1\\n' \"$artifact_size\"\n  fi\nelif [ -e \"$ARTIFACT_PATH\" ]; then\n  printf 'ARTIFACT|{index}|invalid|0|0\\n'\nelse\n  artifact_state=0\n  artifact_parent_state \"$(dirname \"$ARTIFACT_PATH\")\" || artifact_state=$?\n  if [ \"$artifact_state\" -eq 1 ]; then\n    printf 'ARTIFACT|{index}|unconfirmed|0|0\\n'\n  else\n    printf 'ARTIFACT|{index}|missing|0|0\\n'\n  fi\nfi\n",
                size = expected.size,
                digest = shell_quote(&expected.sha256)
            )
        }
        ArtifactKind::Dir => {
            let mut checks = String::new();
            for file in &probe.expected {
                let target = shell_quote(&posix_join(&probe.target, &file.relative));
                checks.push_str(&format!(
                    "    if [ \"$artifact_state\" = present ]; then\n      if [ -L {target} ] || [ ! -f {target} ]; then\n        artifact_state=incomplete\n      else\n        artifact_size=$(artifact_stat {target})\n        artifact_digest=$(artifact_sha256 {target})\n        if [ -z \"$artifact_digest\" ]; then\n          artifact_state=unconfirmed\n        elif [ \"$artifact_size\" != {size} ] || [ \"$artifact_digest\" != {digest} ]; then\n          artifact_state=incomplete\n        fi\n      fi\n    fi\n",
                    size = file.size,
                    digest = shell_quote(&file.sha256)
                ));
            }
            format!(
                r#"ARTIFACT_PATH={path}
if [ -L "$ARTIFACT_PATH" ]; then
  printf 'ARTIFACT|{index}|invalid|0|0\n'
elif [ -d "$ARTIFACT_PATH" ]; then
  if [ ! -r "$ARTIFACT_PATH" ] || [ ! -x "$ARTIFACT_PATH" ]; then
    printf 'ARTIFACT|{index}|unconfirmed|0|0\n'
  elif ! artifact_files=$(find "$ARTIFACT_PATH" -type f 2>/dev/null | wc -l | tr -d ' '); then
    printf 'ARTIFACT|{index}|unconfirmed|0|0\n'
  elif ! artifact_unsafe=$(find "$ARTIFACT_PATH" ! -type f ! -type d -print -quit 2>/dev/null); then
    printf 'ARTIFACT|{index}|unconfirmed|0|%s\n' "$artifact_files"
  elif [ -n "$artifact_unsafe" ]; then
    printf 'ARTIFACT|{index}|invalid|0|%s\n' "$artifact_files"
  else
    artifact_state=present
{checks}    printf 'ARTIFACT|{index}|%s|0|%s\n' "$artifact_state" "$artifact_files"
  fi
elif [ -e "$ARTIFACT_PATH" ]; then
  printf 'ARTIFACT|{index}|invalid|0|0\n'
else
  if artifact_parent_state "$(dirname "$ARTIFACT_PATH")"; then
    printf 'ARTIFACT|{index}|missing|0|0\n'
  else
    printf 'ARTIFACT|{index}|unconfirmed|0|0\n'
  fi
fi
"#
            )
        }
    }
}

#[derive(Debug, Default)]
struct TargetSnapshot {
    checked: bool,
    docker_running: bool,
    compose_available: bool,
    compose_file: String,
    compose_working_dir: String,
    compose: String,
    containers: Vec<TargetContainer>,
    containers_checked: bool,
    target_platform: String,
    image_states: HashMap<usize, ImageSnapshot>,
    artifact_states: HashMap<usize, ArtifactSnapshot>,
    artifacts_checked: bool,
}

#[derive(Debug, Clone, Default)]
struct ImageSnapshot {
    status: String,
    platform: String,
}

#[derive(Debug, Clone, Default)]
struct ArtifactSnapshot {
    status: String,
    actual_bytes: u64,
    actual_files: usize,
}

#[derive(Debug)]
struct TargetContainer {
    name: String,
    image: String,
    state: String,
    health: String,
    service: String,
    working_dir: String,
}

fn parse_target_output(output: &str) -> Result<TargetSnapshot, String> {
    let mut target = TargetSnapshot::default();
    let mut section = "flags";
    for line in output.lines() {
        match line {
            "__REVIEW_COMPOSE_BEGIN__" => {
                section = "compose";
                continue;
            }
            "__REVIEW_COMPOSE_END__" => {
                section = "flags";
                continue;
            }
            "__REVIEW_CONTAINERS_BEGIN__" => {
                section = "containers";
                continue;
            }
            "__REVIEW_CONTAINERS_END__" => {
                section = "flags";
                continue;
            }
            _ => {}
        }
        if section == "compose" {
            target.compose.push_str(line);
            target.compose.push('\n');
        } else if section == "containers" {
            target.containers.push(
                parse_container(line)
                    .ok_or_else(|| "容器状态核对结果不完整，请检查目标主机后重试".to_string())?,
            );
        } else if let Some(rest) = line.strip_prefix("IMAGE|") {
            let fields: Vec<&str> = rest.split('|').collect();
            if fields.len() >= 2 {
                if let Ok(index) = fields[0].parse() {
                    let state = match fields[1] {
                        "ready" if fields.len() >= 3 && valid_image_id(fields[2]) => "ready",
                        "missing" => "missing",
                        "platform_mismatch" => "platform_mismatch",
                        "check_failed" => "check_failed",
                        "unavailable" => "unavailable",
                        "invalid" => "invalid",
                        _ => "check_failed",
                    };
                    target.image_states.insert(
                        index,
                        ImageSnapshot {
                            status: state.to_string(),
                            platform: fields.get(3).copied().unwrap_or_default().to_string(),
                        },
                    );
                }
            }
        } else if let Some(rest) = line.strip_prefix("ARTIFACT|") {
            let fields: Vec<&str> = rest.split('|').collect();
            if fields.len() >= 4 {
                if let Ok(index) = fields[0].parse() {
                    let status = match fields[1] {
                        "present" => "present",
                        "missing" => "missing",
                        "incomplete" => "incomplete",
                        "invalid" => "invalid",
                        _ => "unconfirmed",
                    };
                    target.artifact_states.insert(
                        index,
                        ArtifactSnapshot {
                            status: status.to_string(),
                            actual_bytes: fields[2].parse().unwrap_or(0),
                            actual_files: fields[3].parse().unwrap_or(0),
                        },
                    );
                }
            }
        } else if let Some((key, value)) = line.split_once('=') {
            match key {
                "TARGET_CHECKED" => target.checked = value == "yes",
                "DOCKER_RUNNING" => target.docker_running = value == "yes",
                "COMPOSE_AVAILABLE" => target.compose_available = value == "yes",
                "COMPOSE_FILE" => target.compose_file = value.to_string(),
                "COMPOSE_WORKING_DIR" => target.compose_working_dir = value.to_string(),
                "TARGET_PLATFORM" => target.target_platform = value.to_string(),
                "CONTAINERS_CHECKED" => target.containers_checked = value == "yes",
                "ARTIFACTS_CHECKED" => target.artifacts_checked = value == "yes",
                _ => {}
            }
        }
    }
    if !target.checked
        || section != "flags"
        || (target.docker_running && !target.containers_checked)
    {
        return Err("目标主机未返回完整核对结果，请检查连接后重试".to_string());
    }
    Ok(target)
}

fn parse_container(line: &str) -> Option<TargetContainer> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 6
        || fields[0].trim_start_matches('/').is_empty()
        || !valid_image_reference(fields[1])
        || ![
            "created",
            "restarting",
            "running",
            "removing",
            "paused",
            "exited",
            "dead",
        ]
        .contains(&fields[2])
        || !["none", "healthy", "unhealthy", "starting"].contains(&fields[5])
    {
        return None;
    }
    Some(TargetContainer {
        name: fields[0].trim_start_matches('/').to_string(),
        image: fields[1].to_string(),
        state: fields[2].to_string(),
        service: fields[3].to_string(),
        working_dir: fields[4].to_string(),
        health: fields[5].to_string(),
    })
}

#[cfg(test)]
fn compare(
    compose_file: &str,
    install: &str,
    desired: &str,
    services: &[ComposeServiceView],
    images: &[String],
    target: TargetSnapshot,
) -> Result<DeploymentReview, String> {
    compare_with_artifacts(
        compose_file,
        install,
        desired,
        services,
        images,
        target,
        &[],
    )
}

fn compare_with_artifacts(
    compose_file: &str,
    install: &str,
    desired: &str,
    services: &[ComposeServiceView],
    images: &[String],
    target: TargetSnapshot,
    artifacts: &[ArtifactProbe],
) -> Result<DeploymentReview, String> {
    let compose_present = !target.compose_file.is_empty();
    let compose_working_dir = if !target.compose_working_dir.is_empty() {
        target.compose_working_dir.clone()
    } else if compose_present {
        Path::new(&target.compose_file)
            .parent()
            .unwrap_or_else(|| Path::new(install))
            .to_string_lossy()
            .to_string()
    } else {
        format!("{install}/deploy")
    };
    let compose_services = if compose_present {
        parse_services(&target.compose)
            .map_err(|_| "目标部署目录的 Docker Compose 清单无法解析，请检查现有部署".to_string())?
    } else {
        Vec::new()
    };
    // 同一目录可能存在其他 Compose 项目的残留容器，只有归属于当前清单服务的容器才算已有部署。
    // Compose 文件也可能来自中断的首次部署或人工拷贝；Docker 状态未知时仍按文件判断，
    // 避免在无法核对时误开放首次部署。
    let deployment_containers = target
        .containers
        .iter()
        .filter(|container| {
            same_directory(&container.working_dir, &compose_working_dir)
                && compose_services.iter().any(|service| {
                    container.service == service.name
                        || (!service.container_name.is_empty()
                            && container.name == service.container_name)
                })
        })
        .count();
    let has_deployment = compose_present
        && (!target.docker_running || !target.containers_checked || deployment_containers > 0);
    let current_services = if has_deployment {
        compose_services
    } else {
        Vec::new()
    };
    let desired_doc: serde_yaml::Value = serde_yaml::from_str(desired)
        .map_err(|_| "交付包的 Docker Compose 清单无法解析".to_string())?;
    let current_doc: serde_yaml::Value = if has_deployment {
        serde_yaml::from_str(&target.compose)
            .map_err(|_| "目标 Docker Compose 清单无法解析".to_string())?
    } else {
        serde_yaml::Value::Null
    };
    let mut issues = Vec::new();
    if !target.docker_running {
        issues.push("目标主机的 Docker 不可用，请先完成环境检测并启动 Docker".to_string());
    }
    if !target.compose_available {
        issues.push("目标主机缺少 Docker Compose，请先准备运行环境".to_string());
    }
    if target.docker_running && !target.containers_checked {
        issues.push("容器状态未确认，请检查目标主机后重新核对".to_string());
    }
    // 部署目录由安装根目录与包内清单所在目录拼接而成；比对、预检与执行必须使用同一位置。
    let mut expected_compose_dir = Path::new(install)
        .join(
            Path::new(compose_file)
                .parent()
                .unwrap_or_else(|| Path::new("")),
        )
        .to_string_lossy()
        .to_string();
    while expected_compose_dir.len() > 1 && expected_compose_dir.ends_with('/') {
        expected_compose_dir.pop();
    }
    let supported_update_layout = !compose_present
        || Path::new(&target.compose_file)
            .parent()
            .map(|parent| same_directory(&parent.to_string_lossy(), &expected_compose_dir))
            .unwrap_or(false);
    if !supported_update_layout {
        issues.push(compose_layout_issue(
            &target.compose_file,
            install,
            &expected_compose_dir,
            compose_file,
        ));
    }
    let mut service_reviews = Vec::new();
    for service in services {
        let old = current_services.iter().find(|old| old.name == service.name);
        let container = target.containers.iter().find(|container| {
            (!service.container_name.is_empty() && container.name == service.container_name)
                || (container.service == service.name
                    && same_directory(&container.working_dir, &compose_working_dir))
        });
        if let Some(container) = container {
            if !same_directory(&container.working_dir, &compose_working_dir) {
                issues.push(format!(
                    "容器 {} 已存在，但不属于所选部署目录，请检查目标主机与部署位置",
                    container.name
                ));
            }
        }
        let change = match old {
            None => "added",
            Some(_)
                if desired_doc
                    .get("services")
                    .and_then(|all| all.get(&service.name))
                    == current_doc
                        .get("services")
                        .and_then(|all| all.get(&service.name)) =>
            {
                "unchanged"
            }
            Some(_) => "changed",
        };
        if service.image.is_empty() {
            issues.push(format!(
                "服务 {} 没有可用镜像声明，请补齐交付包清单",
                service.name
            ));
        } else if service.image.contains('$') {
            issues.push(format!(
                "服务 {} 的镜像地址尚未确定，请检查交付包中的环境配置",
                service.name
            ));
        } else if !valid_image_reference(&service.image) {
            issues.push(format!(
                "服务 {} 的镜像地址无效，请检查交付包清单",
                service.name
            ));
        }
        service_reviews.push(DeploymentServiceReview {
            name: service.name.clone(),
            image: service.image.clone(),
            current_image: container
                .map(|c| c.image.clone())
                .or_else(|| old.map(|old| old.image.clone()))
                .unwrap_or_default(),
            container_name: container
                .map(|c| c.name.clone())
                .unwrap_or_else(|| service.container_name.clone()),
            container_state: if target.docker_running {
                container
                    .map(|c| c.state.clone())
                    .unwrap_or_else(|| "not_created".to_string())
            } else {
                "unconfirmed".to_string()
            },
            container_health: container
                .map(|c| c.health.clone())
                .unwrap_or_else(|| "none".to_string()),
            ports: service.ports.clone(),
            depends_on: service.depends_on.clone(),
            profiles: service.profiles.clone(),
            change: change.to_string(),
        });
    }
    for old in &current_services {
        if !services.iter().any(|service| service.name == old.name) {
            service_reviews.push(DeploymentServiceReview {
                name: old.name.clone(),
                image: String::new(),
                current_image: old.image.clone(),
                container_name: old.container_name.clone(),
                container_state: "unconfirmed".to_string(),
                container_health: "none".to_string(),
                ports: old.ports.clone(),
                depends_on: old.depends_on.clone(),
                profiles: old.profiles.clone(),
                change: "removed".to_string(),
            });
        }
    }
    let target_platform = canonical_platform(&target.target_platform);
    let image_reviews: Vec<DeploymentImageReview> = images
        .iter()
        .enumerate()
        .map(|(index, reference)| {
            let snapshot = target.image_states.get(&index);
            let status = if !valid_image_reference(reference) {
                "invalid"
            } else if !target.docker_running {
                "unavailable"
            } else if target.target_platform.is_empty() {
                "check_failed"
            } else {
                snapshot
                    .map(|value| value.status.as_str())
                    .unwrap_or("check_failed")
            };
            let platform = snapshot
                .map(|value| value.platform.clone())
                .unwrap_or_default();
            // 摘要引用的镜像也能按目标架构重新拉取；仓库中该摘要若不含目标架构，
            // Docker 会在拉取时明确提示找不到匹配版本。
            let pullable = status == "missing" || status == "platform_mismatch";
            DeploymentImageReview {
                image_ref: reference.clone(),
                services: services
                    .iter()
                    .filter(|service| service.image == *reference)
                    .map(|service| service.name.clone())
                    .collect(),
                docker_present: matches!(status, "ready" | "platform_mismatch"),
                status: status.to_string(),
                platform: platform.clone(),
                target_platform: target_platform.clone(),
                detail: image_detail(status, &platform, &target_platform, reference),
                pullable,
                local_file: String::new(),
            }
        })
        .collect();
    let missing_images: Vec<String> = image_reviews
        .iter()
        .filter(|image| image.status == "missing")
        .map(|image| image.image_ref.clone())
        .collect();
    let unready_images: Vec<String> = image_reviews
        .iter()
        .filter(|image| image.status != "ready")
        .map(|image| image.image_ref.clone())
        .collect();
    let missing_count = image_reviews
        .iter()
        .filter(|image| image.status == "missing")
        .count();
    let mismatch_count = image_reviews
        .iter()
        .filter(|image| image.status == "platform_mismatch")
        .count();
    let failed_count = image_reviews
        .iter()
        .filter(|image| matches!(image.status.as_str(), "check_failed" | "unavailable"))
        .count();
    let invalid_count = image_reviews
        .iter()
        .filter(|image| image.status == "invalid")
        .count();
    if missing_count > 0 {
        issues.push(format!(
            "目标 Docker 缺少 {missing_count} 个镜像，可在下方拉取或导入。"
        ));
    }
    if mismatch_count > 0 {
        issues.push(format!(
            "{mismatch_count} 个镜像与目标架构不匹配，可在下方重新拉取适配版本。"
        ));
    }
    if failed_count > 0 {
        issues.push(format!(
            "{failed_count} 个镜像状态无法读取，请重新检查；若 Docker 未运行，请先启动 Docker。"
        ));
    }
    if invalid_count > 0 {
        issues.push(format!(
            "{invalid_count} 个镜像地址无效，需修正交付包后重新制作制品。"
        ));
    }
    let artifact_reviews = artifact_reviews(artifacts, &target.artifact_states);
    let mut artifact_issues = Vec::new();
    if has_deployment {
        let missing = artifact_reviews
            .iter()
            .filter(|artifact| artifact.status == "missing")
            .count();
        let incomplete = artifact_reviews
            .iter()
            .filter(|artifact| artifact.status == "incomplete")
            .count();
        let unconfirmed = artifact_reviews
            .iter()
            .filter(|artifact| artifact.status == "unconfirmed")
            .count();
        let invalid = artifact_reviews
            .iter()
            .filter(|artifact| artifact.status == "invalid")
            .count();
        if missing > 0 {
            artifact_issues.push(format!(
                "目标部署缺少 {missing} 个制品文件，下一步增量更新会写入所选组件，请保留对应组件的勾选。"
            ));
        }
        if incomplete > 0 {
            artifact_issues.push(format!(
                "目标部署有 {incomplete} 个制品文件与交付包不一致，下一步增量更新会替换所选组件。"
            ));
        }
        if unconfirmed > 0 {
            artifact_issues.push(format!(
                "有 {unconfirmed} 个制品文件无法核对，请检查目标目录权限后重新检查。"
            ));
        }
        if invalid > 0 {
            artifact_issues.push(format!(
                "有 {invalid} 个制品路径与交付包声明的类型不一致或包含符号链接，请在目标机处理该路径后重新检查。"
            ));
        }
    } else {
        let unconfirmed = artifact_reviews
            .iter()
            .filter(|artifact| artifact.status == "unconfirmed")
            .count();
        let invalid = artifact_reviews
            .iter()
            .filter(|artifact| artifact.status == "invalid")
            .count();
        if unconfirmed > 0 {
            artifact_issues.push(format!(
                "有 {unconfirmed} 个制品文件无法核对，请检查目标目录权限后重新检查。"
            ));
        }
        if invalid > 0 {
            artifact_issues.push(format!(
                "有 {invalid} 个制品路径与交付包声明的类型不一致或包含符号链接，请在目标机处理该路径后重新检查。"
            ));
        }
    }
    let artifact_blocked = artifact_reviews
        .iter()
        .filter(|artifact| matches!(artifact.status.as_str(), "unconfirmed" | "invalid"))
        .count();
    if artifact_blocked > 0 {
        issues.push(format!(
            "有 {artifact_blocked} 个制品文件无法核对或路径异常，请处理目标目录后重新检查。"
        ));
    }
    let ready =
        target.checked && target.docker_running && target.compose_available && issues.is_empty();
    Ok(DeploymentReview {
        compose_file: compose_file.to_string(),
        target_compose_file: target.compose_file,
        current_compose: target.compose,
        desired_compose: desired.to_string(),
        install_dir: install.to_string(),
        has_deployment,
        target_checked: target.checked,
        image_check: "unavailable".to_string(),
        docker_running: target.docker_running,
        compose_available: target.compose_available,
        compose_changed: desired_doc != current_doc,
        services: service_reviews,
        images: image_reviews,
        target_platform,
        artifacts: artifact_reviews,
        artifact_issues,
        missing_images,
        unready_images,
        issues,
        ready,
        can_first_deploy: ready && !has_deployment,
        can_incremental_update: ready && has_deployment && supported_update_layout,
    })
}

fn image_detail(status: &str, platform: &str, target_platform: &str, reference: &str) -> String {
    match status {
        "ready" => String::from("目标 Docker 已具备该镜像"),
        "missing" => String::from("目标 Docker 中没有该镜像，可拉取或导入"),
        "platform_mismatch" if reference.contains('@') => format!(
            "现有镜像为 {platform}，目标为 {target_platform}；可重新拉取目标架构的版本，若该摘要不含目标架构则需更换交付包"
        ),
        "platform_mismatch" => {
            format!("现有镜像为 {platform}，目标为 {target_platform}；可重新拉取适配架构的版本")
        }
        "check_failed" => String::from("读取镜像状态失败，可重新检查"),
        "unavailable" => String::from("Docker 未运行或目标环境未完成检查"),
        "invalid" => String::from("镜像地址无效，需修正交付包"),
        _ => String::from("镜像状态无法读取，请重新检查"),
    }
}

fn artifact_reviews(
    probes: &[ArtifactProbe],
    states: &HashMap<usize, ArtifactSnapshot>,
) -> Vec<DeploymentArtifactReview> {
    probes
        .iter()
        .enumerate()
        .map(|(index, probe)| {
            let snapshot = states.get(&index);
            let status = match snapshot {
                None => "unconfirmed",
                Some(snapshot) if snapshot.status.is_empty() => "unconfirmed",
                Some(snapshot) => snapshot.status.as_str(),
            };
            DeploymentArtifactReview {
                key: probe.key.clone(),
                label: probe.label.clone(),
                source: probe.source.clone(),
                root: probe.root.clone(),
                path: probe.path.clone(),
                target: probe.target.clone(),
                kind: probe.kind.as_str().to_string(),
                status: status.to_string(),
                expected_bytes: probe.expected_bytes,
                actual_bytes: snapshot.map(|value| value.actual_bytes).unwrap_or(0),
                expected_files: probe.expected_files,
                actual_files: snapshot.map(|value| value.actual_files).unwrap_or(0),
            }
        })
        .collect()
}

/// 清单位置与项目部署目录不一致时，给出实际位置与可填写的安装根目录。
fn compose_layout_issue(
    actual_compose: &str,
    install: &str,
    expected_compose_dir: &str,
    compose_file: &str,
) -> String {
    let actual_dir = Path::new(actual_compose)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_default();
    let relative = Path::new(compose_file)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_default();
    let hint = install_root_hint(&actual_dir, &relative)
        .map(|hint| format!("请将「项目管理」中的部署目录改为 {hint} 后重新核对。"))
        .unwrap_or_else(|| {
            format!("请检查「项目管理」中的部署目录，部署清单应位于 {expected_compose_dir}。")
        });
    format!(
        "目标机上的 Compose 清单位于 {actual_dir}，而项目部署目录 {install} 对应的清单位置是 {expected_compose_dir}。{hint}"
    )
}

/// 当前填写的目录正是清单所在目录时，反推出上级安装根目录。
fn install_root_hint(actual_dir: &str, relative: &str) -> Option<String> {
    let dir = actual_dir.trim_end_matches('/');
    let relative = relative.trim().trim_matches('/');
    if relative.is_empty() {
        return None;
    }
    dir.strip_suffix(&format!("/{relative}"))
        .filter(|hint| !hint.is_empty())
        .map(str::to_string)
}

fn same_directory(left: &str, right: &str) -> bool {
    normalize_directory(left) == normalize_directory(right)
}

/// 统一 Docker 标签与本机路径的写法：Windows 扩展前缀、分隔符与大小写。
fn normalize_directory(value: &str) -> String {
    let mut normalized = value.replace('\\', "/");
    if normalized
        .get(..8)
        .map(|prefix| prefix.eq_ignore_ascii_case("//?/UNC/"))
        .unwrap_or(false)
    {
        normalized = format!("//{}", &normalized[8..]);
    } else if let Some(rest) = normalized.strip_prefix("//?/") {
        normalized = rest.to_string();
    }
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    if cfg!(windows) {
        normalized.make_ascii_lowercase();
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(compose: &str, statuses: &[(usize, &str)]) -> TargetSnapshot {
        let mut target = TargetSnapshot {
            checked: true,
            docker_running: true,
            compose_available: true,
            containers_checked: true,
            compose_file: if compose.is_empty() {
                String::new()
            } else {
                "/srv/demo/deploy/docker-compose.yml".to_string()
            },
            compose_working_dir: if compose.is_empty() {
                String::new()
            } else {
                "/srv/demo/deploy".to_string()
            },
            compose: compose.to_string(),
            image_states: statuses
                .iter()
                .map(|(i, status)| {
                    (
                        *i,
                        ImageSnapshot {
                            status: status.to_string(),
                            platform: "linux/amd64".to_string(),
                        },
                    )
                })
                .collect(),
            target_platform: "linux/amd64".to_string(),
            ..TargetSnapshot::default()
        };
        if let Some(service) = parse_services(compose)
            .ok()
            .and_then(|items| items.into_iter().next())
        {
            target.containers.push(TargetContainer {
                name: format!("demo-{}-1", service.name),
                image: service.image,
                state: "running".to_string(),
                health: "healthy".to_string(),
                service: service.name,
                working_dir: target.compose_working_dir.clone(),
            });
        }
        target
    }

    fn file_probe(target: &Path, content: &[u8]) -> ArtifactProbe {
        ArtifactProbe {
            key: "file".to_string(),
            label: "文件".to_string(),
            source: "app/file.txt".to_string(),
            root: "deploy".to_string(),
            path: "file.txt".to_string(),
            kind: ArtifactKind::File,
            target: target.to_string_lossy().to_string(),
            expected_bytes: content.len() as u64,
            expected_files: 1,
            expected: vec![ArtifactExpectedFile {
                relative: String::new(),
                size: content.len() as u64,
                sha256: crate::commands::bundle::sha256_hex(content),
            }],
        }
    }

    fn dir_probe(target: &Path, files: &[(&str, &[u8])]) -> ArtifactProbe {
        ArtifactProbe {
            key: "dir".to_string(),
            label: "目录".to_string(),
            source: "app/dir".to_string(),
            root: "deploy".to_string(),
            path: "dir".to_string(),
            kind: ArtifactKind::Dir,
            target: target.to_string_lossy().to_string(),
            expected_bytes: files.iter().map(|(_, content)| content.len() as u64).sum(),
            expected_files: files.len(),
            expected: files
                .iter()
                .map(|(relative, content)| ArtifactExpectedFile {
                    relative: (*relative).to_string(),
                    size: content.len() as u64,
                    sha256: crate::commands::bundle::sha256_hex(content),
                })
                .collect(),
        }
    }

    #[test]
    fn compares_service_additions_changes_removals_and_configuration() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n    ports: ['8081:80']\n  redis:\n    image: redis:7\n";
        let current = "services:\n  web:\n    image: nginx:1.27\n    ports: ['8080:80']\n  previous:\n    image: nginx:1.26\n";
        let services = parse_services(desired).unwrap();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &unique_images(&services),
            snapshot(current, &[(0, "ready"), (1, "missing")]),
        )
        .unwrap();
        assert_eq!(review.services[0].change, "changed");
        assert_eq!(review.services[1].change, "added");
        assert_eq!(review.services[2].change, "removed");
        assert_eq!(review.current_compose, current);
        assert_eq!(review.desired_compose, desired);
        let serialized = serde_json::to_value(&review).unwrap();
        assert_eq!(serialized["currentCompose"], current);
        assert_eq!(serialized["desiredCompose"], desired);
        assert!(review.has_deployment);
        assert_eq!(review.missing_images, ["redis:7"]);
        assert!(!review.ready);
        assert!(!review.can_incremental_update);
    }

    #[test]
    fn first_deployment_requires_confirmed_docker_and_every_image() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let images = unique_images(&services);
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &images,
            snapshot("", &[(0, "ready")]),
        )
        .unwrap();
        assert!(review.ready);
        assert!(review.can_first_deploy);
        assert!(!review.can_incremental_update);
        let unknown = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &images,
            snapshot("", &[]),
        )
        .unwrap();
        assert!(!unknown.ready);
        assert_eq!(unknown.images[0].status, "check_failed");
    }

    #[test]
    fn digest_reference_is_never_replaced_with_a_tag() {
        let digest = format!("registry.example.com/demo/web:1@sha256:{}", "a".repeat(64));
        let script = target_script("/srv/demo's directory", &[digest.clone()]);
        assert!(script.contains(&format!(
            "docker image inspect --format '{{{{.Id}}}}|{{{{.Os}}}}/{{{{.Architecture}}}}' -- '{}'",
            digest
        )));
        assert!(!script.contains("docker images"));
        assert!(script.contains("INSTALL='/srv/demo'\\''s directory'"));
        assert!(script.contains("normalize_arch()"));
        assert!(script.contains("target_platform=$(docker info"));
    }

    #[test]
    fn image_platform_must_match_the_docker_platform() {
        assert!(platform_matches("linux/amd64", "linux/x86_64"));
        assert!(platform_matches("linux/arm64", "linux/aarch64"));
        assert!(!platform_matches("linux/arm64", "linux/amd64"));
        assert!(!platform_matches("windows/amd64", "linux/amd64"));
        assert!(!platform_matches("", "linux/amd64"));
    }

    #[test]
    fn platform_mismatch_is_explained_and_tag_images_can_be_repaired() {
        let digest_reference = format!("demo/db:1.4.0@sha256:{}", "a".repeat(64));
        let desired = format!(
            "services:\n  web:\n    image: demo/web:1.4.0\n  db:\n    image: '{digest_reference}'\n"
        );
        let services = parse_services(&desired).unwrap();
        let images = unique_images(&services);
        let mut target = snapshot("", &[]);
        target.target_platform = "linux/arm64".to_string();
        for index in 0..images.len() {
            target.image_states.insert(
                index,
                ImageSnapshot {
                    status: "platform_mismatch".to_string(),
                    platform: "linux/amd64".to_string(),
                },
            );
        }
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            &desired,
            &services,
            &images,
            target,
        )
        .unwrap();
        assert_eq!(review.images[0].status, "platform_mismatch");
        assert!(review.images[0].pullable);
        assert!(review.images[0].detail.contains("可重新拉取"));
        assert_eq!(review.images[1].status, "platform_mismatch");
        assert!(review.images[1].pullable);
        assert!(review.images[1].detail.contains("该摘要不含目标架构"));
    }

    #[test]
    fn artifact_reviews_distinguish_missing_size_mismatch_and_present_files() {
        let root = std::env::temp_dir().join(format!(
            "deploy-review-artifacts-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("wrong.txt"), b"short").unwrap();
        fs::write(root.join("ready.txt"), b"ready").unwrap();
        fs::write(root.join("same-size-wrong.txt"), b"other").unwrap();
        let probes = vec![
            ArtifactProbe {
                key: "missing".to_string(),
                label: "缺失".to_string(),
                source: "app/missing.txt".to_string(),
                root: "deploy".to_string(),
                path: "missing.txt".to_string(),
                kind: ArtifactKind::File,
                target: root.join("missing.txt").to_string_lossy().to_string(),
                expected_bytes: 5,
                expected_files: 1,
                expected: vec![ArtifactExpectedFile {
                    relative: String::new(),
                    size: 5,
                    sha256: crate::commands::bundle::sha256_hex(b"ready"),
                }],
            },
            ArtifactProbe {
                key: "wrong".to_string(),
                label: "不一致".to_string(),
                source: "app/wrong.txt".to_string(),
                root: "deploy".to_string(),
                path: "wrong.txt".to_string(),
                kind: ArtifactKind::File,
                target: root.join("wrong.txt").to_string_lossy().to_string(),
                expected_bytes: 10,
                expected_files: 1,
                expected: vec![ArtifactExpectedFile {
                    relative: String::new(),
                    size: 10,
                    sha256: crate::commands::bundle::sha256_hex(b"0123456789"),
                }],
            },
            ArtifactProbe {
                key: "ready".to_string(),
                label: "就绪".to_string(),
                source: "app/ready.txt".to_string(),
                root: "deploy".to_string(),
                path: "ready.txt".to_string(),
                kind: ArtifactKind::File,
                target: root.join("ready.txt").to_string_lossy().to_string(),
                expected_bytes: 5,
                expected_files: 1,
                expected: vec![ArtifactExpectedFile {
                    relative: String::new(),
                    size: 5,
                    sha256: crate::commands::bundle::sha256_hex(b"ready"),
                }],
            },
            ArtifactProbe {
                key: "same-size-wrong".to_string(),
                label: "内容不一致".to_string(),
                source: "app/same-size-wrong.txt".to_string(),
                root: "deploy".to_string(),
                path: "same-size-wrong.txt".to_string(),
                kind: ArtifactKind::File,
                target: root
                    .join("same-size-wrong.txt")
                    .to_string_lossy()
                    .to_string(),
                expected_bytes: 5,
                expected_files: 1,
                expected: vec![ArtifactExpectedFile {
                    relative: String::new(),
                    size: 5,
                    sha256: crate::commands::bundle::sha256_hex(b"ready"),
                }],
            },
        ];
        let states = inspect_local_artifacts(&probes);
        let reviews = artifact_reviews(&probes, &states);
        assert_eq!(
            reviews
                .iter()
                .map(|item| item.status.as_str())
                .collect::<Vec<_>>(),
            vec!["missing", "incomplete", "present", "incomplete"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artifact_directories_need_every_declared_file_with_the_same_content() {
        let root = std::env::temp_dir().join(format!(
            "deploy-review-artifact-dir-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("dir")).unwrap();
        fs::write(root.join("dir/a.txt"), b"alpha").unwrap();
        let probes = vec![dir_probe(&root.join("dir"), &[("a.txt", b"alpha"), ("b.txt", b"beta")])];
        let states = inspect_local_artifacts(&probes);
        assert_eq!(
            artifact_reviews(&probes, &states)[0].status.as_str(),
            "incomplete"
        );

        fs::write(root.join("dir/b.txt"), b"beta").unwrap();
        let states = inspect_local_artifacts(&probes);
        assert_eq!(
            artifact_reviews(&probes, &states)[0].status.as_str(),
            "present"
        );

        fs::write(root.join("dir/b.txt"), b"other").unwrap();
        let states = inspect_local_artifacts(&probes);
        assert_eq!(
            artifact_reviews(&probes, &states)[0].status.as_str(),
            "incomplete"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artifact_type_conflicts_are_reported_as_invalid() {
        let root = std::env::temp_dir().join(format!(
            "deploy-review-artifact-type-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(root.join("conflict")).unwrap();
        let probes = vec![file_probe(&root.join("conflict"), b"ready")];
        let states = inspect_local_artifacts(&probes);
        assert_eq!(
            artifact_reviews(&probes, &states)[0].status.as_str(),
            "invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn artifact_symlinks_are_reported_as_invalid() {
        let root = std::env::temp_dir().join(format!(
            "deploy-review-artifact-link-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("dir")).unwrap();
        fs::write(root.join("dir/a.txt"), b"alpha").unwrap();
        std::os::unix::fs::symlink(root.join("dir/a.txt"), root.join("dir/link.txt")).unwrap();
        let probes = vec![dir_probe(&root.join("dir"), &[("a.txt", b"alpha")])];
        let states = inspect_local_artifacts(&probes);
        assert_eq!(
            artifact_reviews(&probes, &states)[0].status.as_str(),
            "invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_artifact_targets_use_posix_paths() {
        assert_eq!(
            posix_join("/srv/demo", "deploy/docker-compose.yml"),
            "/srv/demo/deploy/docker-compose.yml"
        );
        assert_eq!(posix_join("/srv/demo/", "/deploy/app"), "/srv/demo/deploy/app");
        assert_eq!(posix_join("/srv/demo", ""), "/srv/demo");
    }

    #[test]
    fn parser_separates_compose_from_flags_and_container_metadata() {
        let raw = format!("TARGET_CHECKED=yes\nDOCKER_RUNNING=yes\nCOMPOSE_AVAILABLE=yes\nTARGET_PLATFORM=linux/amd64\nCOMPOSE_FILE=/srv/demo/deploy/docker-compose.yml\n__REVIEW_COMPOSE_BEGIN__\nservices:\n  web:\n    image: nginx:1.27\n__REVIEW_COMPOSE_END__\n__REVIEW_CONTAINERS_BEGIN__\n/demo-web-1\tnginx:1.26\trunning\tweb\t/srv/demo/deploy\thealthy\n__REVIEW_CONTAINERS_END__\nCONTAINERS_CHECKED=yes\nIMAGE|0|ready|sha256:{}|linux/amd64\n", "a".repeat(64));
        let target = parse_target_output(&raw).unwrap();
        assert_eq!(target.containers.len(), 1);
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert_eq!(review.services[0].container_state, "running");
        assert_eq!(review.services[0].current_image, "nginx:1.26");
        assert_eq!(review.services[0].container_health, "healthy");
        assert!(review.ready);
        assert!(!review.can_first_deploy);
    }

    #[test]
    fn parser_keeps_image_platform_and_artifact_probe_results() {
        let raw = format!(
            "TARGET_CHECKED=yes\nDOCKER_RUNNING=yes\nCOMPOSE_AVAILABLE=yes\nCONTAINERS_CHECKED=yes\nTARGET_PLATFORM=linux/arm64\nARTIFACTS_CHECKED=yes\nIMAGE|0|platform_mismatch|sha256:{}|linux/amd64\nARTIFACT|0|present|128|1\n",
            "a".repeat(64)
        );
        let target = parse_target_output(&raw).unwrap();
        assert_eq!(target.target_platform, "linux/arm64");
        assert_eq!(
            target
                .image_states
                .get(&0)
                .map(|snapshot| snapshot.status.as_str()),
            Some("platform_mismatch")
        );
        assert_eq!(
            target
                .image_states
                .get(&0)
                .map(|snapshot| snapshot.platform.as_str()),
            Some("linux/amd64")
        );
        assert_eq!(
            target
                .artifact_states
                .get(&0)
                .map(|artifact| artifact.actual_bytes),
            Some(128)
        );
    }

    #[test]
    fn local_docker_endpoint_follows_context_and_host_precedence() {
        assert!(local_endpoint("unix:///var/run/docker.sock", None, None));
        assert!(local_endpoint("npipe:////./pipe/docker_engine", None, None));
        assert!(!local_endpoint("ssh://remote.example.com", None, None));
        assert!(!local_endpoint(
            "unix:///var/run/docker.sock",
            Some("tcp://remote.example.com:2376"),
            None
        ));
        assert!(local_endpoint(
            "unix:///var/run/docker.sock",
            Some("tcp://remote.example.com:2376"),
            Some("desktop-linux")
        ));
        assert!(!local_endpoint(
            "ssh://remote.example.com",
            Some("unix:///var/run/docker.sock"),
            Some("remote-context")
        ));
    }

    #[test]
    fn compose_preference_and_container_matching_follow_declared_layout() {
        let root = std::env::temp_dir().join(format!(
            "deploy-review-root-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("deploy")).unwrap();
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        fs::write(root.join("deploy/docker-compose.yml"), desired).unwrap();
        let mut target = local_compose_snapshot(&root).unwrap();
        assert_eq!(
            Path::new(&target.compose_file),
            super::super::docker_ops::resolve_compose_file(&root).unwrap()
        );
        target.checked = true;
        target.docker_running = true;
        target.compose_available = true;
        target.containers_checked = true;
        target.target_platform = "linux/amd64".to_string();
        target.image_states.insert(
            0,
            ImageSnapshot {
                status: "ready".to_string(),
                platform: "linux/amd64".to_string(),
            },
        );
        target.containers.push(TargetContainer {
            name: "demo-web-1".to_string(),
            image: "nginx:1.27".to_string(),
            state: "running".to_string(),
            health: "healthy".to_string(),
            service: "web".to_string(),
            working_dir: target.compose_working_dir.clone(),
        });
        let services = parse_services(desired).unwrap();
        let review = compare(
            "deploy/docker-compose.yml",
            root.to_str().unwrap(),
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert!(review.ready);
        assert!(!review.can_first_deploy);
        assert!(review.can_incremental_update);
        assert!(review.issues.is_empty());
        assert_eq!(review.services[0].container_state, "running");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_only_differences_allow_incremental_updates() {
        let current = "services:\n  kafka:\n    image: kafka:4.0\n    environment: { MODE: current }\n    ports: ['9092:9092']\n";
        let desired = "services:\n  kafka:\n    image: kafka:4.0\n    environment: { MODE: updated }\n    ports: ['9093:9092']\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot(current, &[(0, "ready")]);
        target.compose_file = "/srv/demo/deploy/docker-compose.yml".to_string();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert!(review.compose_changed);
        assert_eq!(review.services[0].change, "changed");
        assert_eq!(review.services[0].image, review.services[0].current_image);
        assert!(review.ready);
        assert!(review.can_incremental_update);
        assert!(!review.can_first_deploy);
        assert!(review.issues.is_empty());
    }

    #[test]
    fn compose_file_without_containers_is_treated_as_first_deployment() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot(desired, &[(0, "ready")]);
        target.containers.clear();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert!(review.ready);
        assert!(!review.has_deployment);
        assert!(review.can_first_deploy);
        assert!(!review.can_incremental_update);
    }

    #[test]
    fn stopped_container_still_counts_as_an_existing_deployment() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot(desired, &[(0, "ready")]);
        target.containers[0].state = "exited".to_string();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert!(review.ready);
        assert!(review.has_deployment);
        assert!(review.can_incremental_update);
        assert!(!review.can_first_deploy);
    }

    #[test]
    fn containers_from_other_services_do_not_count_as_the_current_deployment() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot(desired, &[(0, "ready")]);
        target.containers[0].name = "other-api-1".to_string();
        target.containers[0].service = "api".to_string();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert!(review.ready);
        assert!(!review.has_deployment);
        assert!(review.can_first_deploy);
        assert!(!review.can_incremental_update);
    }

    #[test]
    fn zero_container_layout_mismatch_blocks_first_deployment() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot(desired, &[(0, "ready")]);
        target.containers.clear();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo/deploy",
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert!(!review.ready);
        assert!(!review.can_first_deploy);
        assert!(!review.can_incremental_update);
        assert!(review
            .issues
            .iter()
            .any(|issue| issue.contains("部署目录改为 /srv/demo")));
    }

    #[test]
    fn unconfirmed_docker_state_keeps_compose_as_an_existing_deployment() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot(desired, &[(0, "ready")]);
        target.containers.clear();
        target.docker_running = false;
        target.containers_checked = false;
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo",
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert!(!review.ready);
        assert!(review.has_deployment);
        assert!(!review.can_first_deploy);
        assert!(!review.can_incremental_update);
    }

    #[test]
    fn directory_comparison_handles_windows_extended_paths() {
        assert!(same_directory(
            r"\\?\C:\demo\deploy\",
            r"C:\demo\deploy"
        ));
        assert!(same_directory(
            r"\\?\UNC\server\share\deploy",
            r"\\server\share\deploy"
        ));
    }

    #[test]
    fn install_directory_pointing_at_compose_directory_is_blocked_with_hint() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot(desired, &[(0, "ready")]);
        target.compose_file = "/srv/demo/deploy/docker-compose.yml".to_string();
        target.compose_working_dir = "/srv/demo/deploy".to_string();
        let review = compare(
            "deploy/docker-compose.yml",
            "/srv/demo/deploy",
            desired,
            &services,
            &unique_images(&services),
            target,
        )
        .unwrap();
        assert!(!review.ready);
        assert!(!review.can_incremental_update);
        assert!(review
            .issues
            .iter()
            .any(|issue| issue.contains("部署目录改为 /srv/demo")));
    }

    #[test]
    fn unknown_containers_or_invalid_image_refs_cannot_be_ready() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot("", &[(0, "ready")]);
        target.containers_checked = false;
        assert!(
            !compare(
                "deploy/docker-compose.yml",
                "/srv/demo",
                desired,
                &services,
                &unique_images(&services),
                target
            )
            .unwrap()
            .ready
        );
        for reference in ["--help", "https://registry.example.com/web", "nginx image", "nginx@sha256:abc", "nginx;true", "${IMAGE}", "nginx@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@sha256:bbbb"] {
            assert!(!valid_image_reference(reference), "unexpected valid reference: {reference}");
        }
        let invalid = "services:\n  web:\n    image: '--help'\n";
        let services = parse_services(invalid).unwrap();
        assert!(
            !compare(
                "deploy/docker-compose.yml",
                "/srv/demo",
                invalid,
                &services,
                &unique_images(&services),
                snapshot("", &[(0, "ready")])
            )
            .unwrap()
            .ready
        );
        assert!(parse_target_output(
            "TARGET_CHECKED=yes\nDOCKER_RUNNING=yes\nCOMPOSE_AVAILABLE=yes\n"
        )
        .is_err());
    }

    #[test]
    fn compose_v1_alone_cannot_satisfy_execution_requirements() {
        let desired = "services:\n  web:\n    image: nginx:1.27\n";
        let services = parse_services(desired).unwrap();
        let mut target = snapshot("", &[(0, "ready")]);
        target.compose_available = false;
        assert!(
            !compare(
                "deploy/docker-compose.yml",
                "/srv/demo",
                desired,
                &services,
                &unique_images(&services),
                target
            )
            .unwrap()
            .ready
        );
        assert!(!target_script("/srv/demo", &[]).contains("command -v docker-compose"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_probe_script_reads_root_compose_and_checks_complete_image_identity() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "deploy-review-probe-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("deploy")).unwrap();
        fs::write(
            root.join("docker-compose.yml"),
            "services:\n  web:\n    image: nginx:1.27\n",
        )
        .unwrap();
        fs::write(root.join("deploy/docker-compose.lan.yml"), "services: {}\n").unwrap();
        fs::write(root.join("site.txt"), b"payload").unwrap();
        fs::create_dir_all(root.join("site-dir")).unwrap();
        fs::write(root.join("site-dir/one.txt"), b"first").unwrap();
        fs::create_dir_all(root.join("conflict")).unwrap();
        fs::create_dir_all(root.join("linked")).unwrap();
        fs::write(root.join("linked/one.txt"), b"first").unwrap();
        std::os::unix::fs::symlink(root.join("linked/one.txt"), root.join("linked/link.txt")).unwrap();
        fs::create_dir_all(root.join("partial")).unwrap();
        fs::write(root.join("partial/one.txt"), b"first").unwrap();
        let binary = root.join("bin/docker");
        fs::write(
            &binary,
            r#"#!/bin/sh
case "$1" in
  info)
    if [ "$2" = "--format" ]; then printf 'linux/amd64\n'; fi
    exit 0;;
  compose) exit 0;;
  ps) printf 'container-id\n';;
  inspect) printf '/demo-web\tnginx:1.27\trunning\tweb\t%s\thealthy\n' "$REVIEW_TEST_ROOT";;
  image) printf 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|linux/amd64\n';;
  *) exit 1;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![root.join("bin")];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let output = Command::new("bash")
            .arg("-c")
            .arg(target_script_with_artifacts(
                root.to_str().unwrap(),
                &["nginx:1.27".to_string()],
                &[ArtifactProbe {
                    key: "site".to_string(),
                    label: "站点".to_string(),
                    source: "site.txt".to_string(),
                    root: "deploy".to_string(),
                    path: "site.txt".to_string(),
                    kind: ArtifactKind::File,
                    target: root.join("site.txt").to_string_lossy().to_string(),
                    expected_bytes: 7,
                    expected_files: 1,
                    expected: vec![ArtifactExpectedFile {
                        relative: String::new(),
                        size: 7,
                        sha256: crate::commands::bundle::sha256_hex(b"payload"),
                    }],
                }, ArtifactProbe {
                    key: "site-dir".to_string(),
                    label: "站点目录".to_string(),
                    source: "site-dir".to_string(),
                    root: "deploy".to_string(),
                    path: "site-dir".to_string(),
                    kind: ArtifactKind::Dir,
                    target: root.join("site-dir").to_string_lossy().to_string(),
                    expected_bytes: 5,
                    expected_files: 1,
                    expected: vec![ArtifactExpectedFile {
                        relative: "one.txt".to_string(),
                        size: 5,
                        sha256: crate::commands::bundle::sha256_hex(b"first"),
                    }],
                }, ArtifactProbe {
                    key: "missing".to_string(),
                    label: "缺失".to_string(),
                    source: "missing.txt".to_string(),
                    root: "deploy".to_string(),
                    path: "missing.txt".to_string(),
                    kind: ArtifactKind::File,
                    target: root.join("missing.txt").to_string_lossy().to_string(),
                    expected_bytes: 7,
                    expected_files: 1,
                    expected: vec![ArtifactExpectedFile {
                        relative: String::new(),
                        size: 7,
                        sha256: crate::commands::bundle::sha256_hex(b"payload"),
                    }],
                }, ArtifactProbe {
                    key: "conflict".to_string(),
                    label: "类型冲突".to_string(),
                    source: "conflict".to_string(),
                    root: "deploy".to_string(),
                    path: "conflict".to_string(),
                    kind: ArtifactKind::File,
                    target: root.join("conflict").to_string_lossy().to_string(),
                    expected_bytes: 7,
                    expected_files: 1,
                    expected: vec![ArtifactExpectedFile {
                        relative: String::new(),
                        size: 7,
                        sha256: crate::commands::bundle::sha256_hex(b"payload"),
                    }],
                }, ArtifactProbe {
                    key: "linked".to_string(),
                    label: "符号链接".to_string(),
                    source: "linked".to_string(),
                    root: "deploy".to_string(),
                    path: "linked".to_string(),
                    kind: ArtifactKind::Dir,
                    target: root.join("linked").to_string_lossy().to_string(),
                    expected_bytes: 5,
                    expected_files: 1,
                    expected: vec![ArtifactExpectedFile {
                        relative: "one.txt".to_string(),
                        size: 5,
                        sha256: crate::commands::bundle::sha256_hex(b"first"),
                    }],
                }, ArtifactProbe {
                    key: "partial".to_string(),
                    label: "缺少文件".to_string(),
                    source: "partial".to_string(),
                    root: "deploy".to_string(),
                    path: "partial".to_string(),
                    kind: ArtifactKind::Dir,
                    target: root.join("partial").to_string_lossy().to_string(),
                    expected_bytes: 10,
                    expected_files: 2,
                    expected: vec![
                        ArtifactExpectedFile {
                            relative: "one.txt".to_string(),
                            size: 5,
                            sha256: crate::commands::bundle::sha256_hex(b"first"),
                        },
                        ArtifactExpectedFile {
                            relative: "two.txt".to_string(),
                            size: 5,
                            sha256: crate::commands::bundle::sha256_hex(b"other"),
                        },
                    ],
                }],
            ))
            .env("PATH", std::env::join_paths(&paths).unwrap())
            .env("REVIEW_TEST_ROOT", fs::canonicalize(&root).unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let target = parse_target_output(&String::from_utf8_lossy(&output.stdout)).unwrap();
        assert_eq!(
            target.compose_file,
            root.join("docker-compose.yml").to_string_lossy()
        );
        assert_eq!(
            target
                .image_states
                .get(&0)
                .map(|snapshot| snapshot.status.as_str()),
            Some("ready")
        );
        assert_eq!(
            target
                .artifact_states
                .get(&0)
                .map(|artifact| artifact.status.as_str()),
            Some("present")
        );
        assert_eq!(
            target
                .artifact_states
                .get(&0)
                .map(|artifact| artifact.actual_bytes),
            Some(7)
        );
        assert_eq!(
            target
                .artifact_states
                .get(&1)
                .map(|artifact| artifact.status.as_str()),
            Some("present")
        );
        assert_eq!(
            target
                .artifact_states
                .get(&2)
                .map(|artifact| artifact.status.as_str()),
            Some("missing")
        );
        assert_eq!(
            target
                .artifact_states
                .get(&3)
                .map(|artifact| artifact.status.as_str()),
            Some("invalid")
        );
        assert_eq!(
            target
                .artifact_states
                .get(&4)
                .map(|artifact| artifact.status.as_str()),
            Some("invalid")
        );
        assert_eq!(
            target
                .artifact_states
                .get(&5)
                .map(|artifact| artifact.status.as_str()),
            Some("incomplete")
        );
        assert!(target.artifacts_checked);
        assert_eq!(target.containers[0].health, "healthy");
        let reference = format!("demo/web@sha256:{}", "c".repeat(64));
        let binding = DeploymentImageBinding {
            reference: reference.clone(),
            image_id: format!("sha256:{}", "b".repeat(64)),
            platform: "linux/amd64".to_owned(),
        };
        let script = target_script_with_bindings(
            root.to_str().unwrap(),
            &[reference],
            &[],
            &[binding],
        );
        let output = Command::new("bash")
            .args(["-c", &script])
            .env("PATH", std::env::join_paths(&paths).unwrap())
            .env("REVIEW_TEST_ROOT", fs::canonicalize(&root).unwrap())
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let target = parse_target_output(&String::from_utf8_lossy(&output.stdout)).unwrap();
        assert_eq!(target.image_states[&0].status, "check_failed");

        let hash = root.join("bin/sha256sum");
        fs::write(&hash, "#!/bin/sh\nexit 7\n").unwrap();
        fs::set_permissions(&hash, fs::Permissions::from_mode(0o700)).unwrap();
        let script = target_script_with_artifacts(
            root.to_str().unwrap(),
            &[],
            &[file_probe(&root.join("site.txt"), b"payload")],
        );
        let output = Command::new("bash")
            .args(["-c", &script])
            .env("PATH", std::env::join_paths(&paths).unwrap())
            .env("REVIEW_TEST_ROOT", fs::canonicalize(&root).unwrap())
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let target = parse_target_output(&String::from_utf8_lossy(&output.stdout)).unwrap();
        assert_eq!(target.artifact_states[&0].status, "unconfirmed");
        assert!(target.artifacts_checked);
        fs::remove_dir_all(root).unwrap();
    }
}
