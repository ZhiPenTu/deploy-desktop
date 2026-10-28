//! 交付包部署计划：产物映射、依赖镜像、目标机镜像状态与执行步骤。
//!
//! 所有内容都来自 bundle.json 的声明，客户端不内置任何项目专用的目录约定。
use super::bundle::load_target;
use crate::commands::bundle::{ArtifactRoot, ArtifactSpec};
use crate::commands::remote_ssh::{resolve_existing_key, run_ssh_script};
use serde::Serialize;
use std::fs::{self};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFile {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanImage {
    pub image_ref: String,
    pub role: String,
    pub required: bool,
    pub bundle_file: String,
    pub source: String,
    pub present_locally: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanService {
    pub key: String,
    pub label: String,
    pub container: String,
    pub port: Option<u16>,
    pub protocol: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanArtifact {
    pub key: String,
    pub label: String,
    pub source: String,
    pub target: String,
    pub root: String,
    pub kind: String,
    pub size_bytes: u64,
    pub files: Vec<PlanFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanComponent {
    pub key: String,
    pub label: String,
    pub services: Vec<String>,
    pub artifacts: Vec<PlanArtifact>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDatabase {
    pub service_key: String,
    pub name: String,
    pub client: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub key: String,
    pub title: String,
    pub detail: String,
    pub state: String,
    /// 交付包 plan.steps 提供的流程说明，缺失时为空。
    pub hint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundlePlan {
    pub bundle_dir: String,
    pub product: String,
    pub display_name: String,
    pub version: String,
    pub build_number: String,
    pub commit: String,
    pub commit_short: String,
    pub branch: String,
    pub built_at: String,
    pub runtime: String,
    pub compose: String,
    pub variant: String,
    pub components: Vec<PlanComponent>,
    pub services: Vec<PlanService>,
    pub database: Option<PlanDatabase>,
    pub images: Vec<PlanImage>,
    pub image_check: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub steps: Vec<PlanStep>,
}

/// 从交付包推导部署链路：产物、依赖镜像、目标机镜像状态与执行步骤。
///
/// 界面据此渲染「制品 → 交付包 → 镜像 → 容器」，不再由前端猜模式：
/// 包内镜像只是补充，应用镜像由 Compose 清单决定，缺失时提示导入离线镜像。
///
/// 传入 host 时按目标机查询镜像，否则只查本机；查询失败返回 image_check=unavailable，
/// 由界面显示「未确认」，不把未知状态当成缺失。
pub fn inspect_bundle_plan_blocking(
    bundle_dir: String,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<BundlePlan, String> {
    let bundle = load_target(Path::new(bundle_dir.trim()))?;
    let descriptor = &bundle.descriptor;

    let (local_tags, image_check) =
        image_tags_for_target(host.as_deref(), port, user.as_deref(), key_path.as_deref());
    let images = collect_plan_images(&bundle, &local_tags);

    let mut components: Vec<PlanComponent> = Vec::new();
    for component in &descriptor.components {
        let mut artifacts: Vec<PlanArtifact> = Vec::new();
        for key in &component.artifacts {
            let Some(spec) = bundle.artifact(key) else {
                continue;
            };
            artifacts.push(plan_artifact(&bundle.root, spec));
        }
        components.push(PlanComponent {
            key: component.key.clone(),
            label: component.label.clone(),
            services: component.services.clone(),
            artifacts,
        });
    }

    let services = descriptor
        .runtime
        .services
        .iter()
        .map(|service| PlanService {
            key: service.key.clone(),
            label: service.label.clone(),
            container: bundle.container_name(&service.key),
            port: service.port,
            protocol: if service.protocol.trim().is_empty() {
                "TCP".to_string()
            } else {
                service.protocol.clone()
            },
            description: service.description.clone(),
        })
        .collect::<Vec<_>>();

    let file_count = descriptor.files.len();
    let total_bytes = descriptor.files.iter().map(|file| file.size).sum();

    let missing_required = images
        .iter()
        .filter(|image| image.required && !image.present_locally)
        .count();
    let images_detail = if images.is_empty() {
        "交付包未声明镜像需求".to_string()
    } else if image_check == "unavailable" {
        format!("{} 个镜像，目标机状态未确认", images.len())
    } else if missing_required == 0 {
        format!("{} 个镜像已就位", images.len())
    } else {
        format!(
            "{} 个镜像中 {} 个在{}缺失",
            images.len(),
            missing_required,
            check_label(&image_check)
        )
    };

    // 目标环境实际运行的是声明的变体清单（如局域网覆盖文件），这里展示同一份文件。
    let compose_in_use = bundle.effective_compose();

    let has_offline_images = bundle.has_bundle_images();
    let mut steps = vec![
        PlanStep {
            key: "artifact".to_string(),
            title: "获取交付包".to_string(),
            detail: format!("版本 {} 已解包，{file_count} 个文件", descriptor.metadata.version),
            state: "ready".to_string(),
            hint: String::new(),
        },
        PlanStep {
            key: "images".to_string(),
            title: if has_offline_images {
                "导入随包镜像".to_string()
            } else {
                "准备基础镜像".to_string()
            },
            detail: images_detail,
            state: if missing_required > 0 && image_check != "unavailable" {
                "blocked".to_string()
            } else if image_check == "unavailable" {
                "optional".to_string()
            } else {
                "ready".to_string()
            },
            hint: String::new(),
        },
        PlanStep {
            key: "distribute".to_string(),
            title: "分发交付包".to_string(),
            detail: format!(
                "{} 个产物，{} 个文件",
                descriptor.artifacts.len(),
                file_count
            ),
            state: if descriptor.artifacts.is_empty() {
                "blocked".to_string()
            } else {
                "ready".to_string()
            },
            hint: String::new(),
        },
        PlanStep {
            key: "start".to_string(),
            title: "启动容器栈".to_string(),
            detail: format!("docker compose -f {compose_in_use} up -d"),
            state: "ready".to_string(),
            hint: String::new(),
        },
        PlanStep {
            key: "verify".to_string(),
            title: "验证与验收".to_string(),
            detail: "健康检查、登录与页面验证".to_string(),
            state: "ready".to_string(),
            hint: String::new(),
        },
    ];
    apply_plan_steps(&descriptor.plan.steps, &mut steps);

    Ok(BundlePlan {
        bundle_dir: bundle.root.to_string_lossy().to_string(),
        product: descriptor.metadata.product.clone(),
        display_name: descriptor.metadata.display_name.clone(),
        version: descriptor.metadata.version.clone(),
        build_number: descriptor.metadata.build_number.clone(),
        commit: descriptor.metadata.commit.clone(),
        commit_short: descriptor.metadata.commit_short.clone(),
        branch: descriptor.metadata.branch.clone(),
        built_at: descriptor.metadata.built_at.clone(),
        runtime: descriptor.runtime.runtime_type.clone(),
        compose: compose_in_use.to_string(),
        variant: descriptor.runtime.variant.clone(),
        components,
        services,
        database: descriptor
            .runtime
            .database
            .as_ref()
            .map(|database| PlanDatabase {
                service_key: database.service_key.clone(),
                name: database.name.clone(),
                client: database.client.clone(),
                port: database.port,
            }),
        images,
        image_check,
        file_count,
        total_bytes,
        steps,
    })
}

#[tauri::command]
pub async fn inspect_bundle_plan(
    bundle_dir: String,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<BundlePlan, String> {
    tauri::async_runtime::spawn_blocking(move || {
        inspect_bundle_plan_blocking(bundle_dir, host, port, user, key_path)
    })
    .await
    .map_err(|error| format!("读取交付包失败：{error}"))?
}

fn check_label(image_check: &str) -> &'static str {
    match image_check {
        "remote" => "目标机",
        "local" => "本机",
        _ => "目标机",
    }
}

fn plan_artifact(root: &Path, spec: &ArtifactSpec) -> PlanArtifact {
    let source = root.join(spec.source.trim());
    let mut files: Vec<PlanFile> = Vec::new();
    if spec.kind == crate::commands::bundle::ArtifactKind::File {
        let size_bytes = fs::metadata(&source).map(|meta| meta.len()).unwrap_or(0);
        files.push(PlanFile {
            path: spec.source.trim().to_string(),
            size_bytes,
        });
    } else {
        collect_plan_files(root, &source, &mut files);
        files.sort_by(|left, right| left.path.cmp(&right.path));
    }
    let size_bytes = files.iter().map(|file| file.size_bytes).sum();
    PlanArtifact {
        key: spec.key.clone(),
        label: spec.label.clone(),
        source: spec.source.trim().to_string(),
        target: spec.path.trim().to_string(),
        root: match spec.root {
            ArtifactRoot::Install => "install".to_string(),
            ArtifactRoot::Deploy => "deploy".to_string(),
        },
        kind: spec.kind.as_str().to_string(),
        size_bytes,
        files,
    }
}

/// 交付包声明的步骤可覆盖标题与补充说明；未内置的步骤按声明顺序追加。
fn apply_plan_steps(declared: &[crate::commands::bundle::PlanStepSpec], steps: &mut Vec<PlanStep>) {
    for entry in declared {
        match steps.iter_mut().find(|step| step.key == entry.key) {
            Some(step) => {
                if !entry.title.trim().is_empty() {
                    step.title = entry.title.trim().to_string();
                }
                step.hint = entry.hint.trim().to_string();
            }
            None => steps.push(PlanStep {
                key: entry.key.clone(),
                title: if entry.title.trim().is_empty() {
                    entry.key.clone()
                } else {
                    entry.title.trim().to_string()
                },
                detail: entry.hint.trim().to_string(),
                state: "ready".to_string(),
                hint: entry.hint.trim().to_string(),
            }),
        }
    }
}

fn collect_plan_files(root: &Path, dir: &Path, out: &mut Vec<PlanFile>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_plan_files(root, &path, out);
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let size_bytes = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            out.push(PlanFile {
                path: relative,
                size_bytes,
            });
        }
    }
}

/// 汇总交付包声明的镜像：images[] 为准，缺少声明时兜底解析 Compose 的 image 行。
fn collect_plan_images(bundle: &crate::commands::bundle::Bundle, local: &[String]) -> Vec<PlanImage> {
    let mut entries: Vec<(String, String, String, bool, String)> = Vec::new();

    for image in &bundle.descriptor.images {
        entries.push((
            image.reference.clone(),
            match image.role {
                crate::commands::bundle::ImageRole::App => "app".to_string(),
                crate::commands::bundle::ImageRole::Base => "base".to_string(),
            },
            if image.bundle_file.trim().is_empty() {
                "declared".to_string()
            } else {
                "bundle".to_string()
            },
            image.required,
            image.bundle_file.trim().to_string(),
        ));
    }

    if entries.is_empty() {
        let compose_path = bundle.root.join(bundle.effective_compose());
        if let Ok(text) = fs::read_to_string(&compose_path) {
            for reference in crate::core::compose::scan_image_references(&text) {
                entries.push((
                    reference,
                    "app".to_string(),
                    "compose".to_string(),
                    true,
                    String::new(),
                ));
            }
        }
    }

    entries.sort();
    entries.dedup_by(|left, right| left.0 == right.0);
    entries
        .into_iter()
        .map(|(reference, role, source, required, bundle_file)| PlanImage {
            present_locally: image_present(local, &reference),
            image_ref: reference,
            role,
            required,
            bundle_file,
            source,
        })
        .collect()
}

/// 查询镜像列表：给了 host 就查目标机，否则查本机。
/// 返回 (镜像标签, "remote"|"local"|"unavailable")。
pub(super) fn image_tags_for_target(
    host: Option<&str>,
    port: Option<u16>,
    user: Option<&str>,
    key_path: Option<&str>,
) -> (Vec<String>, String) {
    let host = host.unwrap_or_default().trim();
    if host.is_empty() {
        return (
            local_image_tags(),
            if docker_available() {
                "local"
            } else {
                "unavailable"
            }
            .to_string(),
        );
    }

    let user = user.unwrap_or("root");
    let key = resolve_existing_key(key_path.map(|value| value.to_string()), host)
        .ok()
        .flatten();
    let Ok(output) = run_ssh_script(
        host,
        port.unwrap_or(22),
        user,
        key.as_deref(),
        "docker images --format '{{.Repository}}:{{.Tag}}' 2>/dev/null || true",
    ) else {
        return (Vec::new(), "unavailable".to_string());
    };
    if !output.status.success() {
        return (Vec::new(), "unavailable".to_string());
    }
    let tags = parse_image_tags(&output.stdout);
    if tags.is_empty() {
        // 目标机可能还没装 Docker：空列表同样视为「未确认」。
        return (tags, "unavailable".to_string());
    }
    (tags, "remote".to_string())
}

pub(super) fn docker_available() -> bool {
    crate::commands::docker_setup::docker_command()
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn local_image_tags() -> Vec<String> {
    let Ok(output) = crate::commands::docker_setup::docker_command()
        .args(["images", "--format", "{{.Repository}}:{{.Tag}}"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_image_tags(&output.stdout)
}

fn parse_image_tags(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty() && line != "<none>:<none>")
        .collect()
}

pub(super) fn image_present(local: &[String], reference: &str) -> bool {
    let without_digest = reference.split('@').next().unwrap_or(reference);
    local
        .iter()
        .any(|tag| tag == reference || tag == without_digest)
}
