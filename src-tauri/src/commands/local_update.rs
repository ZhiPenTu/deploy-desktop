//! 本机增量更新。预检只读取文件与 Docker，执行阶段不删除数据卷。
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

use super::bundle::{ArtifactRoot, Bundle, IMAGE_COMPONENT};
use super::remote_ssh::find_binary;
use super::remote_update::RemoteUpdateProgress;

#[derive(Debug, Serialize)]
pub struct LocalUpdateResult {
    pub success: bool,
    pub components: Vec<String>,
    pub backup_dir: Option<String>,
    pub output: String,
    pub error: Option<String>,
    pub duration_secs: u64,
}

#[derive(Debug)]
struct Replacement {
    source: PathBuf,
    relative: PathBuf,
}

struct UpdatePlan {
    root: PathBuf,
    deploy_dir: PathBuf,
    compose: PathBuf,
    bundle_compose: PathBuf,
    components: Vec<String>,
    replacements: Vec<Replacement>,
    images: Vec<PathBuf>,
    services: Vec<String>,
    all_services: Vec<String>,
    /// 每个所选组件的服务与实际部署路径，用于校验产物是否真的被容器挂载。
    bindings: Vec<ComponentBinding>,
    config_selected: bool,
    image_selected: bool,
}

struct ComponentBinding {
    services: Vec<String>,
    paths: Vec<PathBuf>,
}

#[tauri::command]
pub async fn update_local_deployment(
    window: tauri::Window,
    bundle_dir: String,
    install_dir: String,
    components: Vec<String>,
    backup: bool,
    dry_run: bool,
) -> Result<LocalUpdateResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_update(
            &window,
            &bundle_dir,
            &install_dir,
            components,
            backup,
            dry_run,
        )
    })
    .await
    .map_err(|_| "本机更新任务执行失败，请重试".to_string())?
}

fn progress(window: &tauri::Window, step: &str, percent: usize, status: &str, message: &str) {
    let _ = window.emit(
        "remote-update-progress",
        RemoteUpdateProgress {
            step: step.to_string(),
            percent,
            status: status.to_string(),
            message: message.to_string(),
        },
    );
}

fn run_update(
    window: &tauri::Window,
    bundle_dir: &str,
    install_dir: &str,
    components: Vec<String>,
    backup: bool,
    dry_run: bool,
) -> Result<LocalUpdateResult, String> {
    static LOCAL_UPDATE_LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCAL_UPDATE_LOCK
        .try_lock()
        .map_err(|_| "已有本机更新任务正在进行，请稍后重试".to_string())?;
    let started = Instant::now();
    progress(
        window,
        "检查本机部署",
        5,
        "running",
        "检查交付包、部署目录与所选服务",
    );
    let mut plan = build_plan(bundle_dir, install_dir, &components)?;
    let components = plan.components.clone();
    check_local_docker()?;
    let current = compose_config(&plan, &plan.compose)?;
    // 未选择的配置不参与预检，避免无关差异阻断产物更新。
    let incoming = if plan.config_selected {
        compose_config(&plan, &plan.bundle_compose)?
    } else {
        current.clone()
    };
    validate_effective_configs(&plan, &current, &incoming)?;
    if compose_output(&plan, &plan.compose, &["ps", "-a", "-q"])?
        .trim()
        .is_empty()
    {
        return Err("目标目录尚未创建任何部署服务，请先完成首次部署".to_string());
    }
    let effective_config = if plan.config_selected {
        &incoming
    } else {
        &current
    };
    plan.all_services = active_services(effective_config);
    // 查询归属于当前 Compose 项目的容器，避免误操作同名的其他项目。
    for service in &plan.services {
        let ids = compose_output(&plan, &plan.compose, &["ps", "-a", "-q", service])?;
        if ids.trim().is_empty() {
            return Err(format!("目标部署尚未创建服务 {service}，请先完成首次部署"));
        }
    }
    if backup {
        check_backup_space(&plan)?;
    }
    if !plan.image_selected {
        check_images_available(effective_config)?;
    }
    if dry_run {
        progress(
            window,
            "预检通过",
            100,
            "success",
            "交付包与本机部署检查通过，未修改部署文件或服务",
        );
        return Ok(LocalUpdateResult {
            success: true, components, backup_dir: None,
            output: "预检通过：部署目录、Compose 配置、所选服务、镜像与备份空间检查通过。未修改部署文件，也未重启服务。".to_string(),
            error: None, duration_secs: started.elapsed().as_secs(),
        });
    }

    let backup_dir = if backup {
        progress(window, "创建备份", 15, "running", "备份即将替换的部署文件");
        Some(create_backup(&plan)?)
    } else {
        None
    };
    let execution = (|| -> Result<(), String> {
        for (index, item) in plan.replacements.iter().enumerate() {
            progress(
                window,
                "替换部署文件",
                25 + index * 30 / plan.replacements.len().max(1),
                "running",
                "写入所选组件与配置",
            );
            let destination = plan.root.join(&item.relative);
            validate_destination(&plan.root, &destination)?;
            replace_path(&item.source, &destination)?;
        }
        for image in &plan.images {
            progress(
                window,
                "导入镜像",
                60,
                "running",
                "将交付包镜像导入本机 Docker",
            );
            docker_output(&["load", "-i", &image.to_string_lossy()], "导入镜像")?;
        }
        let effective = if plan.config_selected {
            preferred_compose(&plan.deploy_dir)?
        } else {
            plan.compose.clone()
        };
        check_images_available(&compose_config(&plan, &effective)?)?;
        progress(
            window,
            "应用更新",
            75,
            "running",
            "重启受影响的服务并等待就绪",
        );
        if plan.config_selected || plan.image_selected {
            // 禁止联网拉取或隐式构建，离线更新仅使用预检/导入的镜像。
            compose_output(
                &plan,
                &effective,
                &["up", "-d", "--pull", "never", "--no-build"],
            )?;
        }
        if !plan.services.is_empty() {
            let mut args = vec!["restart"];
            args.extend(plan.services.iter().map(String::as_str));
            compose_output(&plan, &effective, &args)?;
        }
        let waiting = if plan.config_selected || plan.image_selected {
            &plan.all_services
        } else {
            &plan.services
        };
        wait_for_services(&plan, &effective, waiting)?;
        Ok(())
    })();
    let error = execution.err();
    let success = error.is_none();
    progress(
        window,
        if success {
            "更新完成"
        } else {
            "更新失败"
        },
        100,
        if success { "success" } else { "failed" },
        if success {
            "所选服务已更新并通过就绪检查"
        } else {
            "更新未完成，请查看结果与备份目录"
        },
    );
    Ok(LocalUpdateResult {
        success,
        components,
        backup_dir: backup_dir.map(|path| path.to_string_lossy().to_string()),
        output: if success {
            "本机更新完成，所选服务已通过就绪检查。"
        } else if backup {
            "本机更新未完成，已停止后续操作。可使用备份恢复替换前的文件。"
        } else {
            "本机更新未完成，已停止后续操作。请检查容器日志和部署文件。"
        }
        .to_string(),
        error,
        duration_secs: started.elapsed().as_secs(),
    })
}

fn existing_directory(raw: &str, label: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw.trim());
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(format!("{label}必须使用不含上级跳转的绝对路径"));
    }
    let canonical = fs::canonicalize(path).map_err(|_| format!("{label}不存在或无法访问"))?;
    if !canonical.is_dir() || canonical.parent().is_none() {
        return Err(format!("{label}无效，请选择具体目录"));
    }
    // macOS 的 /var、/tmp 是系统别名；固定根目录后始终使用真实路径。
    // 用户传入的目录本身仍不可为符号链接。
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(true)
    {
        return Err(format!("{label}不能使用符号链接，请选择实际目录"));
    }
    reject_symlinks(&canonical)?;
    Ok(canonical)
}

fn reject_symlinks(path: &Path) -> Result<(), String> {
    let mut cursor = PathBuf::new();
    for part in path.components() {
        cursor.push(part);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("部署路径含符号链接，请选择实际目录".to_string())
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("无法检查部署路径，请检查访问权限".to_string()),
        }
    }
    Ok(())
}

fn validate_tree(path: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "无法读取交付文件，请检查目录权限".to_string())?;
    if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
        return Err("交付包或部署目录包含符号链接或特殊文件，无法安全更新".to_string());
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(|_| "无法读取部署目录".to_string())? {
            validate_tree(&entry.map_err(|_| "无法读取部署文件".to_string())?.path())?;
        }
    }
    Ok(())
}

fn validate_destination(root: &Path, path: &Path) -> Result<(), String> {
    if !path.starts_with(root)
        || path == root
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("更新目标超出部署目录，已中止操作".to_string());
    }
    reject_symlinks(path)?;
    if path.exists() {
        validate_tree(path)?;
    }
    Ok(())
}

fn preferred_compose(dir: &Path) -> Result<PathBuf, String> {
    ["docker-compose.lan.yml", "docker-compose.yml"]
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| "未找到 Compose 清单，请检查交付包和部署目录".to_string())
}

fn service_names(compose: &Path) -> Result<BTreeSet<String>, String> {
    let text = fs::read_to_string(compose).map_err(|_| "无法读取 Compose 清单".to_string())?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text)
        .map_err(|_| "Compose 清单格式错误，请检查交付包".to_string())?;
    let services = value
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
        .ok_or_else(|| "Compose 清单未声明服务".to_string())?;
    let names: BTreeSet<String> = services
        .keys()
        .filter_map(serde_yaml::Value::as_str)
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return Err("Compose 清单未声明服务".to_string());
    }
    Ok(names)
}

fn build_plan(
    bundle_dir: &str,
    install_dir: &str,
    components: &[String],
) -> Result<UpdatePlan, String> {
    let bundle_root = existing_directory(bundle_dir, "交付包目录")?;
    let root = existing_directory(install_dir, "部署目录")?;
    if bundle_root.starts_with(&root) || root.starts_with(&bundle_root) {
        return Err("交付包与部署目录不能重叠，请先将交付包解压到独立目录".to_string());
    }

    let bundle = Bundle::load(&bundle_root)?;
    bundle.ensure_compatible()?;
    let components = super::bundle::normalize_components(&bundle, components.to_vec())?;

    let deploy_dir = root.join(bundle.deploy_source_rel());
    if deploy_dir != root {
        validate_destination(&root, &deploy_dir)?;
    }
    let compose_spec = bundle
        .artifact_for_source(&bundle.descriptor.runtime.compose)
        .ok_or_else(|| "交付包未声明 Compose 清单，请重新获取制品".to_string())?;
    let compose = deploy_dir.join(compose_spec.path.trim());
    if !compose.is_file() {
        return Err("未找到 Compose 清单，请检查部署目录".to_string());
    }
    let bundle_compose = bundle.root.join(bundle.descriptor.runtime.compose.trim());

    reject_symlinks(&compose)?;
    reject_symlinks(&bundle_compose)?;
    validate_tree(bundle_compose.parent().unwrap_or(&bundle.root))?;
    let current = service_names(&compose)?;
    let incoming = service_names(&bundle_compose)?;
    if current.is_disjoint(&incoming) {
        return Err("交付包与当前部署的服务不匹配，请确认所选主机和制品".to_string());
    }

    let config_source = bundle.descriptor.runtime.compose.trim().to_string();
    let config_selected = components.iter().any(|key| {
        bundle
            .component(key)
            .map(|component| component.artifacts.iter().any(|artifact| {
                bundle
                    .artifact(artifact)
                    .map(|spec| spec.source == config_source)
                    .unwrap_or(false)
            }))
            .unwrap_or(false)
    });
    let image_selected = components.iter().any(|key| key == IMAGE_COMPONENT);

    let mut replacements: Vec<Replacement> = Vec::new();
    let mut bindings: Vec<ComponentBinding> = Vec::new();
    let mut services = BTreeSet::new();
    let mut images = Vec::new();

    for key in &components {
        if key == IMAGE_COMPONENT {
            let files = bundle.image_files();
            if files.is_empty() {
                return Err("交付包不含可导入的离线镜像，请取消镜像更新或选择完整制品".to_string());
            }
            for path in files {
                validate_tree(&path)?;
                if fs::metadata(&path)
                    .map(|metadata| metadata.len() == 0)
                    .unwrap_or(true)
                {
                    return Err("离线镜像包为空，请重新获取完整制品".to_string());
                }
                images.push(path);
            }
            continue;
        }
        let component = bundle
            .component(key)
            .ok_or_else(|| "交付包未声明所选组件，请重新选择制品".to_string())?;
        let mut targets: Vec<PathBuf> = Vec::new();
        for artifact_key in &component.artifacts {
            let spec = bundle
                .artifact(artifact_key)
                .ok_or_else(|| "交付包未声明所选产物，请重新获取制品".to_string())?;
            let source = bundle.root.join(spec.source.trim());
            if !source.exists() {
                return Err(format!(
                    "交付包缺少所选组件文件：{}",
                    spec.source.trim()
                ));
            }
            reject_symlinks(&source)?;
            validate_tree(&source)?;
            if source.is_file()
                && fs::metadata(&source)
                    .map(|metadata| metadata.len() == 0)
                    .unwrap_or(true)
            {
                return Err("交付包中的组件文件为空，请重新获取完整制品".to_string());
            }
            if source.is_dir()
                && fs::read_dir(&source)
                    .map(|mut entries| entries.next().is_none())
                    .unwrap_or(true)
            {
                return Err("交付包中的组件目录为空，请重新获取完整制品".to_string());
            }
            let base = match spec.root {
                ArtifactRoot::Install => root.clone(),
                ArtifactRoot::Deploy => deploy_dir.clone(),
            };
            let destination = base.join(spec.path.trim());
            validate_destination(&root, &destination)?;
            if !targets.iter().any(|path| path == &destination) {
                targets.push(destination.clone());
            }
            let relative = destination
                .strip_prefix(&root)
                .map_err(|_| "更新目标超出部署目录，已中止操作".to_string())?
                .to_path_buf();
            if replacements.iter().any(|item| item.relative == relative) {
                continue;
            }
            replacements.push(Replacement { source, relative });
        }
        for service in &component.services {
            if !current.contains(service) || !incoming.contains(service) {
                return Err(format!(
                    "所选组件的服务 {service} 与交付包或当前部署不匹配，请检查部署设置"
                ));
            }
            services.insert(service.clone());
        }
        bindings.push(ComponentBinding {
            services: component.services.clone(),
            paths: targets,
        });
    }

    images.sort();
    images.dedup();
    Ok(UpdatePlan {
        root,
        deploy_dir,
        compose,
        bundle_compose,
        components,
        replacements,
        images,
        services: services.into_iter().collect(),
        all_services: incoming.into_iter().collect(),
        bindings,
        config_selected,
        image_selected,
    })
}

fn docker_output(args: &[&str], purpose: &str) -> Result<String, String> {
    let output = Command::new(find_binary("docker"))
        .args(args)
        .output()
        .map_err(|_| "无法运行 Docker，请确认已安装并启动 Docker".to_string())?;
    if !output.status.success() {
        return Err(format!(
            "{purpose}失败，请检查 Docker 状态、部署配置与目录权限"
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn check_local_docker() -> Result<(), String> {
    let context = docker_output(
        &[
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ],
        "读取 Docker 连接",
    )?;
    // 显式 DOCKER_CONTEXT 的优先级高于 DOCKER_HOST，与 Docker CLI 保持一致。
    let endpoint = if std::env::var("DOCKER_CONTEXT")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        context
    } else {
        std::env::var("DOCKER_HOST")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(context)
    };
    let endpoint = endpoint.trim();
    if !endpoint.starts_with("unix://") && !endpoint.starts_with("npipe://") {
        return Err("本机更新需要连接本机 Docker，请先切换 Docker 连接".to_string());
    }
    docker_output(&["info", "--format", "{{.ServerVersion}}"], "检查 Docker")?;
    docker_output(&["compose", "version"], "检查 Docker Compose")?;
    Ok(())
}

fn compose_command(plan: &UpdatePlan, compose: &Path, action: &[&str]) -> Result<Command, String> {
    let mut command = Command::new(find_binary("docker"));
    command
        .arg("compose")
        .arg("--project-directory")
        .arg(&plan.deploy_dir);
    for env in super::docker_ops::resolve_env_files(&plan.root) {
        reject_symlinks(&env)?;
        command.arg("--env-file").arg(env);
    }
    command.arg("-f").arg(compose);
    command.args(action).current_dir(&plan.deploy_dir);
    Ok(command)
}

fn compose_output(plan: &UpdatePlan, compose: &Path, action: &[&str]) -> Result<String, String> {
    let output = compose_command(plan, compose, action)?
        .output()
        .map_err(|_| "无法运行 Docker Compose，请确认 Docker 已启动".to_string())?;
    if !output.status.success() {
        return Err("Compose 操作失败，请检查镜像、配置和目录权限".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn compose_config(plan: &UpdatePlan, compose: &Path) -> Result<Value, String> {
    serde_json::from_str(&compose_output(
        plan,
        compose,
        &["config", "--format", "json"],
    )?)
    .map_err(|_| "无法解析 Compose 配置，请检查 Docker Compose 版本".to_string())
}

fn validate_effective_configs(
    plan: &UpdatePlan,
    current: &Value,
    incoming: &Value,
) -> Result<(), String> {
    if plan.config_selected
        && current.get("name").and_then(Value::as_str) != incoming.get("name").and_then(Value::as_str)
    {
        return Err("交付包的 Compose 项目名称与当前部署不同，请确认制品与目标".to_string());
    }
    for binding in &plan.bindings {
        let configurations: Vec<&Value> = if plan.config_selected {
            vec![incoming, current]
        } else {
            vec![current]
        };
        for service in &binding.services {
            for path in &binding.paths {
                for config in &configurations {
                    let bound = config
                        .get("services")
                        .and_then(|value| value.get(service))
                        .and_then(|value| value.get("volumes"))
                        .and_then(Value::as_array)
                        .map(|volumes| {
                            volumes.iter().any(|volume| {
                                volume.get("type").and_then(Value::as_str) == Some("bind")
                                    && volume
                                        .get("source")
                                        .and_then(Value::as_str)
                                        .map(|source| path.starts_with(Path::new(source)))
                                        .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);
                    if !bound {
                        return Err(format!(
                            "服务 {service} 未挂载所选产物目录，无法安全增量更新，请检查部署清单"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn check_images_available(config: &Value) -> Result<(), String> {
    let services = config
        .get("services")
        .and_then(Value::as_object)
        .ok_or_else(|| "Compose 清单缺少服务".to_string())?;
    for (name, service) in services {
        if service
            .get("profiles")
            .and_then(Value::as_array)
            .map(|profiles| !profiles.is_empty())
            .unwrap_or(false)
        {
            continue;
        }
        let image = service
            .get("image")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("服务 {name} 未声明离线镜像，无法执行更新"))?;
        if let Some((_, digest)) = image.split_once('@') {
            let hex = digest.strip_prefix("sha256:").unwrap_or_default();
            if hex.len() != 64
                || !hex
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(format!("服务 {name} 的镜像摘要无效，请重新获取制品"));
            }
        }
        docker_output(
            &["image", "inspect", "--format", "{{.Id}}", "--", image],
            &format!("检查服务 {name} 的离线镜像"),
        )?;
    }
    Ok(())
}

fn active_services(config: &Value) -> Vec<String> {
    config
        .get("services")
        .and_then(Value::as_object)
        .map(|services| {
            services
                .iter()
                .filter(|(_, service)| {
                    service
                        .get("profiles")
                        .and_then(Value::as_array)
                        .map(|profiles| profiles.is_empty())
                        .unwrap_or(true)
                })
                .map(|(name, _)| name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn tree_bytes(path: &Path) -> Result<u64, String> {
    if !path.exists() {
        return Ok(0);
    }
    let metadata = fs::metadata(path).map_err(|_| "无法计算备份大小".to_string())?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut size = 0u64;
    for entry in fs::read_dir(path).map_err(|_| "无法计算备份大小".to_string())? {
        size = size.saturating_add(tree_bytes(
            &entry.map_err(|_| "无法计算备份大小".to_string())?.path(),
        )?);
    }
    Ok(size)
}

fn check_backup_space(plan: &UpdatePlan) -> Result<(), String> {
    let required = plan
        .replacements
        .iter()
        .try_fold(64 * 1024 * 1024u64, |total, item| {
            Ok::<u64, String>(
                total
                    .saturating_add(tree_bytes(&plan.root.join(&item.relative))?)
                    .saturating_add(tree_bytes(&item.source)?),
            )
        })?;
    #[cfg(unix)]
    {
        let output = Command::new("df")
            .arg("-Pk")
            .arg(&plan.root)
            .output()
            .map_err(|_| "无法检查部署分区空间".to_string())?;
        let text = String::from_utf8_lossy(&output.stdout);
        let available = text
            .lines()
            .last()
            .and_then(|line| line.split_whitespace().nth(3))
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| "无法检查部署分区空间".to_string())?
            * 1024;
        if !output.status.success() || available < required {
            return Err("部署分区空间不足，请清理后再更新".to_string());
        }
    }
    #[cfg(windows)]
    {
        // 命令是固定文本，不把用户路径插入 PowerShell 表达式。
        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "Get-PSDrive -PSProvider FileSystem | Select-Object Root,Free | ConvertTo-Json -Compress"])
            .output().map_err(|_| "无法检查部署分区空间".to_string())?;
        if !output.status.success() {
            return Err("无法检查部署分区空间".to_string());
        }
        let drives: Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| "无法检查部署分区空间".to_string())?;
        let entries: Vec<&Value> = match drives.as_array() {
            Some(entries) => entries.iter().collect(),
            None => vec![&drives],
        };
        let available = entries
            .iter()
            .find_map(|drive| {
                let drive_root = drive.get("Root")?.as_str()?;
                plan.root
                    .to_string_lossy()
                    .trim_start_matches(r"\\?\")
                    .to_lowercase()
                    .starts_with(&drive_root.to_lowercase())
                    .then(|| drive.get("Free").and_then(Value::as_u64))
                    .flatten()
            })
            .ok_or_else(|| "无法检查部署分区空间".to_string())?;
        if available < required {
            return Err("部署分区空间不足，请清理后再更新".to_string());
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    validate_tree(source)?;
    if source.is_dir() {
        fs::create_dir_all(target).map_err(|_| "创建更新目录失败，请检查权限".to_string())?;
        for entry in fs::read_dir(source).map_err(|_| "读取更新目录失败".to_string())? {
            let entry = entry.map_err(|_| "读取更新文件失败".to_string())?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| "创建更新目录失败，请检查权限".to_string())?;
        }
        fs::copy(source, target)
            .map_err(|_| "复制更新文件失败，请检查空间和目录权限".to_string())?;
    }
    Ok(())
}

fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn create_backup(plan: &UpdatePlan) -> Result<PathBuf, String> {
    let parent = plan.root.join("backup");
    validate_destination(&plan.root, &parent)?;
    fs::create_dir_all(&parent).map_err(|_| "创建备份目录失败，请检查权限".to_string())?;
    let backup = parent.join(format!("update-{}", unique_suffix()));
    fs::create_dir(&backup).map_err(|_| "创建备份目录失败".to_string())?;
    for item in &plan.replacements {
        let existing = plan.root.join(&item.relative);
        if existing.exists() {
            copy_tree(&existing, &backup.join(&item.relative))?;
        }
    }
    Ok(backup)
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn replace_path(source: &Path, target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "更新目标目录无效".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "创建更新目录失败，请检查权限".to_string())?;
    let suffix = unique_suffix();
    let staging = parent.join(format!(".deploy-update-new-{suffix}"));
    let previous = parent.join(format!(".deploy-update-old-{suffix}"));
    if let Err(error) = copy_tree(source, &staging) {
        let _ = remove_path(&staging);
        return Err(error);
    }
    let had_previous = target.exists();
    if had_previous && fs::rename(target, &previous).is_err() {
        let _ = remove_path(&staging);
        return Err("替换原部署文件失败，请检查目录权限".to_string());
    }
    if fs::rename(&staging, target).is_err() {
        if had_previous {
            let _ = fs::rename(&previous, target);
        }
        let _ = remove_path(&staging);
        return Err("写入更新失败，已尝试恢复原文件，请检查部署目录".to_string());
    }
    if had_previous {
        remove_path(&previous)
            .map_err(|_| "更新文件已写入，但无法清理临时旧文件，请检查目录权限".to_string())?;
    }
    Ok(())
}

fn wait_for_services(plan: &UpdatePlan, compose: &Path, services: &[String]) -> Result<(), String> {
    for service in services {
        let mut ready = false;
        for _ in 0..45 {
            let ids = compose_output(plan, compose, &["ps", "-a", "-q", service])?;
            let ids: Vec<&str> = ids.split_whitespace().collect();
            if !ids.is_empty() {
                ready = ids.iter().all(|id| docker_output(&["inspect", "--format", "{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}", id], "检查服务状态")
                    .map(|state| state.trim() == "running|healthy" || state.trim() == "running|none").unwrap_or(false));
            }
            if ready {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if !ready {
            return Err(format!("服务 {service} 在等待时间内未就绪，请检查容器日志"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::bundle::sha256_hex;
    use serde_json::json;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("deploy-local-update-test-{}", unique_suffix()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, relative: &str, text: impl AsRef<[u8]>) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fn spec(&self, dir: &str, compose: &str) -> BundleSpec {
            BundleSpec::new(self.0.join(dir), compose)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 按目录实际内容生成 v2 交付包，避免测试手写文件摘要。
    struct BundleSpec {
        root: PathBuf,
        compose: String,
        services: Vec<serde_json::Value>,
        artifacts: Vec<serde_json::Value>,
        components: Vec<serde_json::Value>,
        images: Vec<serde_json::Value>,
    }

    impl BundleSpec {
        fn new(root: PathBuf, compose: &str) -> Self {
            Self {
                root,
                compose: compose.to_string(),
                services: Vec::new(),
                artifacts: Vec::new(),
                components: Vec::new(),
                images: Vec::new(),
            }
        }

        fn service(mut self, key: &str) -> Self {
            self.services.push(json!({
                "key": key, "label": key, "port": 8080, "protocol": "TCP", "description": ""
            }));
            self
        }

        fn artifact(mut self, key: &str, source: &str, root: &str, path: &str, kind: &str) -> Self {
            self.artifacts.push(json!({
                "key": key, "label": key, "source": source, "root": root, "path": path, "type": kind
            }));
            self
        }

        fn component(mut self, key: &str, artifacts: &[&str], services: &[&str]) -> Self {
            self.components.push(json!({
                "key": key, "label": key,
                "artifacts": artifacts.to_vec(), "services": services.to_vec()
            }));
            self
        }

        fn write(self) -> PathBuf {
            let mut files = Vec::new();
            collect(&self.root, &self.root, &mut files);
            files.sort_by(|left: &serde_json::Value, right: &serde_json::Value| {
                left["path"].as_str().unwrap_or_default().cmp(right["path"].as_str().unwrap_or_default())
            });
            let manifest = json!({
                "apiVersion": "deploy.desktop/bundle/v2",
                "kind": "DeploymentBundle",
                "metadata": { "product": "demo", "displayName": "演示", "version": "1-a",
                    "commit": "deadbeef", "commitShort": "deadbee", "branch": "main", "builtAt": "" },
                "runtime": { "type": "compose", "compose": &self.compose,
                    "containerPrefix": "demo", "services": self.services },
                "components": self.components,
                "artifacts": self.artifacts,
                "images": self.images,
                "plan": { "steps": [] },
                "files": files,
            });
            fs::write(
                self.root.join("bundle.json"),
                serde_json::to_string_pretty(&manifest).unwrap(),
            )
            .unwrap();
            self.root
        }
    }

    fn collect(root: &Path, dir: &Path, out: &mut Vec<serde_json::Value>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, out);
                continue;
            }
            let relative = path.strip_prefix(root).unwrap().to_string_lossy().to_string();
            if relative == "bundle.json" {
                continue;
            }
            let bytes = fs::read(&path).unwrap();
            out.push(json!({
                "path": relative,
                "size": bytes.len(),
                "sha256": sha256_hex(&bytes),
            }));
        }
    }

    fn compose_text(mode: &str) -> String {
        format!("services:\n  frontend:\n    image: nginx\n    environment: {{ MODE: {mode} }}\n")
    }

    #[test]
    fn rejects_overlapping_or_unrelated_deployments_without_writes() {
        let fixture = Fixture::new();
        fixture.write("target/deploy/docker-compose.yml", "services:\n  other:\n    image: nginx\n");
        fixture.write("bundle/deploy/docker-compose.yml", &compose_text("incoming"));
        let bundle_dir = fixture
            .spec("bundle", "deploy/docker-compose.yml")
            .service("frontend")
            .artifact("compose", "deploy/docker-compose.yml", "deploy", "docker-compose.yml", "file")
            .component("config", &["compose"], &[])
            .write();
        let error = build_plan(
            bundle_dir.to_str().unwrap(),
            fixture.0.join("target").to_str().unwrap(),
            &["config".to_string()],
        )
        .err()
        .unwrap();
        assert!(error.contains("不匹配"));
        assert!(!fixture.0.join("target/backup").exists());
        let error = build_plan(
            fixture.0.join("target").to_str().unwrap(),
            fixture.0.join("target").to_str().unwrap(),
            &["config".to_string()],
        )
        .err()
        .unwrap();
        assert!(error.contains("不能重叠"));
    }

    #[test]
    fn follows_descriptor_targets_without_project_specific_paths() {
        let compose = "services:\n  api:\n    image: nginx\n  web:\n    image: nginx\n";
        let fixture = Fixture::new();
        fixture.write("target/api/site/index.html", "old");
        fixture.write("target/bin/tool", "old bin");
        fixture.write("target/api/docker-compose.yml", compose);
        fixture.write("bundle/app/web/index.html", "new");
        fixture.write("bundle/app/bin/tool", "new bin");
        fixture.write("bundle/api/docker-compose.yml", compose);
        let bundle_dir = fixture
            .spec("bundle", "api/docker-compose.yml")
            .service("api")
            .service("web")
            .artifact("site", "app/web", "deploy", "site", "dir")
            .artifact("bin", "app/bin", "install", "bin", "dir")
            .artifact("compose", "api/docker-compose.yml", "deploy", "docker-compose.yml", "file")
            .component("site", &["site"], &["web"])
            .component("cli", &["bin"], &[])
            .component("config", &["compose"], &[])
            .write();
        let plan = build_plan(
            bundle_dir.to_str().unwrap(),
            fixture.0.join("target").to_str().unwrap(),
            &["site".to_string(), "cli".to_string()],
        )
        .unwrap();
        assert_eq!(plan.deploy_dir, fs::canonicalize(fixture.0.join("target/api")).unwrap());
        let relative: Vec<String> = plan
            .replacements
            .iter()
            .map(|item| item.relative.to_string_lossy().to_string())
            .collect();
        assert!(relative.contains(&"api/site".to_string()));
        assert!(relative.contains(&"bin".to_string()));
        for item in &plan.replacements {
            replace_path(&item.source, &plan.root.join(&item.relative)).unwrap();
        }
        assert_eq!(fs::read_to_string(fixture.0.join("target/api/site/index.html")).unwrap(), "new");
        assert_eq!(fs::read_to_string(fixture.0.join("target/bin/tool")).unwrap(), "new bin");
        assert_eq!(fs::read_to_string(fixture.0.join("target/api/docker-compose.yml")).unwrap(), compose);
    }

    #[test]
    fn backups_preserve_configuration_and_replacement_removes_stale_files() {
        let fixture = Fixture::new();
        for folder in ["target", "bundle"] {
            fixture.write(&format!("{folder}/deploy/docker-compose.yml"), &compose_text("current"));
        }
        fixture.write("target/deploy/nginx/stale.conf", "old");
        fixture.write("bundle/deploy/nginx/new.conf", "new");
        let bundle_dir = fixture
            .spec("bundle", "deploy/docker-compose.yml")
            .service("frontend")
            .artifact("compose", "deploy/docker-compose.yml", "deploy", "docker-compose.yml", "file")
            .artifact("nginx", "deploy/nginx", "deploy", "nginx", "dir")
            .component("config", &["compose", "nginx"], &[])
            .write();
        let plan = build_plan(
            bundle_dir.to_str().unwrap(),
            fixture.0.join("target").to_str().unwrap(),
            &["config".to_string()],
        )
        .unwrap();
        let backup = create_backup(&plan).unwrap();
        assert!(backup.join("deploy/docker-compose.yml").is_file());
        assert_eq!(
            fs::read_to_string(backup.join("deploy/nginx/stale.conf")).unwrap(),
            "old"
        );
        for item in &plan.replacements {
            replace_path(&item.source, &plan.root.join(&item.relative)).unwrap();
        }
        assert!(!fixture.0.join("target/deploy/nginx/stale.conf").exists());
        assert_eq!(
            fs::read_to_string(fixture.0.join("target/deploy/nginx/new.conf")).unwrap(),
            "new"
        );
    }

    #[test]
    fn artifact_only_plan_preserves_customized_configuration() {
        for layout in ["", "deploy/"] {
            let fixture = Fixture::new();
            let compose_source = format!("{layout}docker-compose.yml");
            fixture.write(&format!("target/{layout}docker-compose.yml"), &compose_text("customized"));
            fixture.write(&format!("bundle/{layout}docker-compose.yml"), &compose_text("incoming"));
            fixture.write("bundle/app/frontend-dist/index.html", "new page");
            let bundle_dir = fixture
                .spec("bundle", &compose_source)
                .service("frontend")
                .artifact("compose", &compose_source, "deploy", "docker-compose.yml", "file")
                .artifact("dist", "app/frontend-dist", "deploy", "frontend_dist", "dir")
                .component("frontend", &["dist"], &["frontend"])
                .component("config", &["compose"], &[])
                .write();
            let plan = build_plan(
                bundle_dir.to_str().unwrap(),
                fixture.0.join("target").to_str().unwrap(),
                &["frontend".to_string()],
            )
            .unwrap();
            assert_eq!(plan.replacements.len(), 1);
            assert!(!plan.config_selected);
            for item in &plan.replacements {
                replace_path(&item.source, &plan.root.join(&item.relative)).unwrap();
            }
            assert_eq!(
                fs::read_to_string(&plan.compose).unwrap(),
                compose_text("customized")
            );
            assert_eq!(
                fs::read_to_string(plan.deploy_dir.join("frontend_dist/index.html")).unwrap(),
                "new page"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_sources_and_destination_parents() {
        let fixture = Fixture::new();
        fixture.write("outside/file", "keep");
        fs::create_dir_all(fixture.0.join("target")).unwrap();
        std::os::unix::fs::symlink(fixture.0.join("outside"), fixture.0.join("target/deploy"))
            .unwrap();
        assert!(validate_destination(
            &fixture.0.join("target"),
            &fixture.0.join("target/deploy/file")
        )
        .is_err());
        assert!(validate_tree(&fixture.0.join("target")).is_err());
        assert_eq!(
            fs::read_to_string(fixture.0.join("outside/file")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn rejects_path_traversal_in_install_dir() {
        assert!(existing_directory("/tmp/../tmp", "目录").is_err());
        assert!(existing_directory("relative/dir", "目录").is_err());
    }

    #[test]
    fn ignores_unselected_config_but_rejects_project_changes_and_wrong_artifact_mounts() {
        let fixture = Fixture::new();
        let root = fs::canonicalize(&fixture.0).unwrap();
        let destination = root.join("deploy/frontend_dist");
        let plan = UpdatePlan {
            deploy_dir: root.join("deploy"),
            root,
            compose: PathBuf::new(),
            bundle_compose: PathBuf::new(),
            components: vec!["frontend".to_string()],
            replacements: Vec::new(),
            images: Vec::new(),
            services: vec!["frontend".to_string()],
            all_services: Vec::new(),
            bindings: vec![ComponentBinding {
                services: vec!["frontend".to_string()],
                paths: vec![destination.clone()],
            }],
            config_selected: false,
            image_selected: false,
        };
        let current = serde_json::json!({
            "name": "same-project",
            "services": { "frontend": { "volumes": [{ "type": "bind", "source": destination }] } }
        });
        assert!(validate_effective_configs(&plan, &current, &current).is_ok());
        let mut other_project = current.clone();
        other_project["name"] = Value::String("another-project".to_string());
        assert!(validate_effective_configs(&plan, &current, &other_project).is_ok());

        let mut config_plan = UpdatePlan {
            deploy_dir: plan.deploy_dir.clone(),
            root: plan.root.clone(),
            compose: PathBuf::new(),
            bundle_compose: PathBuf::new(),
            components: vec!["config".to_string()],
            replacements: Vec::new(),
            images: Vec::new(),
            services: Vec::new(),
            all_services: Vec::new(),
            bindings: Vec::new(),
            config_selected: true,
            image_selected: false,
        };
        assert!(validate_effective_configs(&config_plan, &current, &other_project).is_err());
        config_plan.config_selected = false;

        let without_bind =
            serde_json::json!({ "name": "same-project", "services": { "frontend": {} } });
        assert!(validate_effective_configs(&plan, &without_bind, &current).is_err());
    }

    #[test]
    fn disabled_profile_services_do_not_block_default_stack_health() {
        let config = serde_json::json!({ "services": {
            "frontend": { "image": "nginx" },
            "maintenance": { "image": "tool", "profiles": ["maintenance"] }
        } });
        assert_eq!(active_services(&config), vec!["frontend"]);
    }
}
