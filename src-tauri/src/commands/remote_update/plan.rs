use super::bundle::inspect_bundle_layout;
use crate::commands::remote_ssh::{resolve_existing_key, run_ssh_script};
use serde::Serialize;
use std::fs::{self};
use std::path::{Path, PathBuf};
use std::process::Command;

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
    pub source: String,
    pub present_locally: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub key: String,
    pub title: String,
    pub detail: String,
    pub state: String,
    /// 交付包 deploy/deployment-plan.json 提供的流程说明，缺失时为空。
    pub hint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundlePlan {
    pub bundle_dir: String,
    pub product: String,
    pub mode: String,
    pub version: String,
    pub commit: String,
    pub commit_short: String,
    pub branch: String,
    pub built_at: String,
    pub components: Vec<String>,
    pub app_files: Vec<PlanFile>,
    pub config_files: Vec<PlanFile>,
    pub images: Vec<PlanImage>,
    pub image_check: String,
    pub steps: Vec<PlanStep>,
}

#[derive(Default)]
struct BundleMeta {
    product: String,
    mode: String,
    version: String,
    commit: String,
    commit_short: String,
    branch: String,
    built_at: String,
    components: Vec<String>,
}

/// 从交付包推导部署链路：产物、依赖镜像、目标机镜像状态与执行步骤。
///
/// 界面据此渲染「制品 → 交付包 → 镜像 → 容器」，不再由前端猜模式：
/// 文件模式的镜像只是基础镜像（可选），镜像模式的应用镜像必须全部就位才能启动。
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
    let root = PathBuf::from(bundle_dir.trim());
    if !root.is_dir() {
        return Err(format!("交付包目录不存在: {}", root.display()));
    }

    let layout = inspect_bundle_layout(bundle_dir.clone())?;
    let meta = read_bundle_meta(&root);
    let mode = if meta.mode.is_empty() {
        layout.mode.clone()
    } else {
        meta.mode.clone()
    };
    let app_files = collect_plan_files(&root, &root.join("app"));
    let config_files = collect_plan_files(&root, &root.join("deploy"));

    let (local_tags, image_check) =
        image_tags_for_target(host.as_deref(), port, user.as_deref(), key_path.as_deref());
    let images = collect_plan_images(&root, &local_tags);

    let is_file = mode == "file";
    let missing = images.iter().filter(|image| !image.present_locally).count();
    let images_detail = if images.is_empty() {
        "交付包未声明镜像需求".to_string()
    } else if image_check == "unavailable" {
        format!("{} 个镜像，目标机状态未确认", images.len())
    } else if missing == 0 {
        format!("{} 个镜像均已在{}", images.len(), check_label(&image_check))
    } else {
        format!(
            "{} 个镜像中 {} 个在{}缺失",
            images.len(),
            missing,
            check_label(&image_check)
        )
    };

    let mut steps = vec![
        PlanStep {
            key: "artifact".to_string(),
            title: "获取交付包".to_string(),
            detail: if meta.version.is_empty() {
                "已解包到本地目录".to_string()
            } else {
                format!("版本 {} 已解包", meta.version)
            },
            state: "ready".to_string(),
            hint: String::new(),
        },
        PlanStep {
            key: "images".to_string(),
            title: if is_file {
                "准备基础镜像".to_string()
            } else {
                "导入应用镜像".to_string()
            },
            detail: images_detail,
            state: if is_file {
                if missing == 0 && image_check != "unavailable" {
                    "ready".to_string()
                } else {
                    "optional".to_string()
                }
            } else if missing > 0 {
                "blocked".to_string()
            } else {
                "ready".to_string()
            },
            hint: String::new(),
        },
        PlanStep {
            key: "distribute".to_string(),
            title: "分发交付包".to_string(),
            detail: format!(
                "app/ {} 个文件，deploy/ {} 个文件",
                app_files.len(),
                config_files.len()
            ),
            state: if app_files.is_empty() && config_files.is_empty() {
                "blocked".to_string()
            } else {
                "ready".to_string()
            },
            hint: String::new(),
        },
        PlanStep {
            key: "start".to_string(),
            title: "启动容器栈".to_string(),
            detail: "docker compose up -d".to_string(),
            state: if layout.has_compose {
                "ready".to_string()
            } else {
                "blocked".to_string()
            },
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

    apply_plan_overrides(&root, &mut steps);

    Ok(BundlePlan {
        bundle_dir: bundle_dir.trim().to_string(),
        product: meta.product,
        mode,
        version: meta.version,
        commit: meta.commit,
        commit_short: meta.commit_short,
        branch: meta.branch,
        built_at: meta.built_at,
        components: meta.components,
        app_files,
        config_files,
        images,
        image_check,
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

fn read_bundle_meta(root: &Path) -> BundleMeta {
    let Ok(text) = fs::read_to_string(root.join("bundle.json")) else {
        return BundleMeta::default();
    };
    let Ok(meta) = serde_json::from_str::<serde_json::Value>(&text) else {
        return BundleMeta::default();
    };
    let pick = |key: &str| {
        meta.get(key)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let components = meta
        .get("components")
        .and_then(|value| value.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|item| item.as_str().map(|value| value.to_string()))
                .collect()
        })
        .unwrap_or_default();
    BundleMeta {
        product: pick("product"),
        mode: pick("mode"),
        version: pick("version"),
        commit: pick("commit"),
        commit_short: pick("commitShort"),
        branch: pick("branch"),
        built_at: pick("builtAt"),
        components,
    }
}

/// 预制流程说明：交付包内 deploy/deployment-plan.json 可覆盖步骤标题并补充说明，
/// 步骤顺序、实时数量与状态仍由程序按实际文件推导，避免清单写死后与实际交付漂移。
fn apply_plan_overrides(root: &Path, steps: &mut [PlanStep]) {
    let Ok(text) = fs::read_to_string(root.join("deploy/deployment-plan.json")) else {
        return;
    };
    let Ok(plan) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let Some(list) = plan.get("steps").and_then(|value| value.as_array()) else {
        return;
    };
    for entry in list {
        let key = entry
            .get("key")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let Some(step) = steps.iter_mut().find(|step| step.key == key) else {
            continue;
        };
        if let Some(title) = entry.get("title").and_then(|value| value.as_str()) {
            if !title.trim().is_empty() {
                step.title = title.trim().to_string();
            }
        }
        if let Some(hint) = entry.get("hint").and_then(|value| value.as_str()) {
            step.hint = hint.trim().to_string();
        }
    }
}

fn collect_plan_files(root: &Path, dir: &Path) -> Vec<PlanFile> {
    let mut files = Vec::new();
    collect_plan_files_into(root, dir, &mut files);
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

fn collect_plan_files_into(root: &Path, dir: &Path, out: &mut Vec<PlanFile>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_plan_files_into(root, &path, out);
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let size_bytes = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            out.push(PlanFile {
                path: relative,
                size_bytes,
            });
        }
    }
}

/// 汇总交付包声明的镜像：
///   1. images/images.json（镜像模式，流水线生成）
///   2. deploy/base-images.txt（文件模式，仓库写死的基础镜像清单）
///   3. 兜底解析 deploy/docker-compose.yml 的 image 行
fn collect_plan_images(root: &Path, local: &[String]) -> Vec<PlanImage> {
    let mut entries: Vec<(String, String)> = Vec::new();

    if let Ok(text) = fs::read_to_string(root.join("images/images.json")) {
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(files) = meta.get("files").and_then(|value| value.as_array()) {
                for entry in files {
                    if let Some(list) = entry.get("images").and_then(|value| value.as_array()) {
                        for image in list {
                            if let Some(reference) = image.as_str() {
                                entries.push((reference.to_string(), "bundle".to_string()));
                            }
                        }
                    }
                }
            }
        }
    }

    let declared = fs::read_to_string(root.join("deploy/base-images.txt")).unwrap_or_default();
    for line in declared.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        entries.push((line.to_string(), "declared".to_string()));
    }

    if entries.is_empty() {
        let compose =
            fs::read_to_string(root.join("deploy/docker-compose.yml")).unwrap_or_default();
        for line in compose.lines() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed.strip_prefix("image:") else {
                continue;
            };
            let reference = rest.trim().trim_matches('"').to_string();
            if !reference.is_empty() {
                entries.push((reference, "compose".to_string()));
            }
        }
    }

    entries.sort();
    entries.dedup();
    // 同一个镜像同时有 :latest 与构建号标签时，只保留 :latest，避免界面重复。
    let refs: Vec<String> = entries
        .iter()
        .map(|(reference, _)| reference.clone())
        .collect();
    entries.retain(|(reference, _)| {
        let base = reference
            .rsplit_once(':')
            .map(|(head, _)| head)
            .unwrap_or(reference);
        let has_latest = refs.iter().any(|other| other == &format!("{base}:latest"));
        !has_latest || reference.ends_with(":latest")
    });

    entries
        .into_iter()
        .map(|(reference, source)| {
            let present_locally = image_present(local, &reference);
            PlanImage {
                image_ref: reference,
                source,
                present_locally,
            }
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
    Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn local_image_tags() -> Vec<String> {
    let Ok(output) = Command::new("docker")
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
