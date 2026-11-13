//! Full-version local deployment with previous-version recovery.
//!
//! This path intentionally stops the whole Compose project before taking file and
//! data-volume snapshots. A deployment either commits the complete bundle or restores
//! the previous version; partial component updates are no longer accepted here.

use super::bundle::{
    default_components, sha256_path, write_deployed_record, ArtifactRoot, Bundle,
    DeployedComposeFile, DeployedImage, DeployedRecord, DEPLOYED_RECORD_DIR, DEPLOYED_RECORD_FILE,
};
use super::deployment_tx::{
    self, DeploymentLock, DeploymentTransaction, ImageSnapshot, VolumeSnapshot, PHASE_APPLYING,
    PHASE_COMMITTED, PHASE_PREPARING, PHASE_RECOVERING, PHASE_SNAPSHOTTING_DATA,
    PHASE_SNAPSHOTTING_FILES, PHASE_STARTING,
};
use crate::core::{path_guard, process};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tauri::Emitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDeployResult {
    pub success: bool,
    pub recovered: bool,
    pub recovery_failed: bool,
    pub stage: String,
    pub version: String,
    pub previous_version: String,
    pub transaction_id: String,
    pub snapshot_dir: String,
    pub error: Option<String>,
    pub output: String,
    pub duration_secs: u64,
    pub status: Option<DeploymentTransaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentTransactionView {
    pub transaction: Option<DeploymentTransaction>,
}

struct FullDeployPlan {
    install_root: PathBuf,
    deploy_dir: PathBuf,
    bundle: Bundle,
    components: Vec<String>,
    managed_paths: Vec<String>,
    retired_paths: Vec<String>,
    new_compose: PathBuf,
    new_compose_rel: String,
    old_compose: Option<PathBuf>,
    old_compose_rel: String,
    had_deployment: bool,
    previous_version: String,
    project_name: String,
    incoming_config: Value,
    old_images: Vec<ImageSnapshot>,
    volumes: Vec<VolumePlan>,
    services: Vec<String>,
    old_services: Vec<String>,
    frontend_service: String,
    frontend_port: Option<u16>,
    required_backup_bytes: u64,
    env_content: Option<String>,
    project_id: String,
}

#[derive(Debug, Clone)]
struct VolumePlan {
    logical_name: String,
    image: String,
    actual_name: String,
    exists: bool,
    bytes: u64,
}

#[tauri::command]
pub async fn deploy_local_version(
    window: tauri::Window,
    bundle_dir: String,
    install_dir: String,
    dry_run: bool,
) -> Result<VersionDeployResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_full_deploy(&window, &bundle_dir, &install_dir, dry_run)
    })
    .await
    .map_err(|error| format!("完整版本部署任务执行失败：{error}"))?
}

#[tauri::command]
pub async fn deploy_remote_version(
    window: tauri::Window,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
    install_dir: String,
    dry_run: bool,
) -> Result<VersionDeployResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        super::remote_update::deploy_remote_full(
            &window,
            host,
            port,
            user,
            key_path,
            bundle_dir,
            install_dir,
            dry_run,
        )
    })
    .await
    .map_err(|error| format!("远程完整版本部署任务执行失败：{error}"))?
}

#[tauri::command]
pub async fn inspect_deployment_transaction(
    install_dir: String,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<DeploymentTransactionView, String> {
    let remote = host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty());
    if let Some(host) = remote {
        let host = host.to_string();
        let user = user.unwrap_or_else(|| "root".to_string());
        return tauri::async_runtime::spawn_blocking(move || {
            super::remote_update::inspect_remote_transaction(
                &host,
                port.unwrap_or(22),
                &user,
                key_path.as_deref(),
                &install_dir,
            )
            .map(|transaction| DeploymentTransactionView { transaction })
        })
        .await
        .map_err(|error| format!("读取部署事务失败：{error}"))?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let install = resolve_install_root(&install_dir, false)?;
        Ok(DeploymentTransactionView {
            transaction: if install.exists() {
                deployment_tx::latest_transaction(&install)?
            } else {
                None
            },
        })
    })
    .await
    .map_err(|error| format!("读取部署事务失败：{error}"))?
}

#[tauri::command]
pub async fn recover_deployment_transaction(
    window: tauri::Window,
    install_dir: String,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<VersionDeployResult, String> {
    let remote = host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty());
    if let Some(host) = remote {
        let host = host.to_string();
        let user = user.unwrap_or_else(|| "root".to_string());
        return tauri::async_runtime::spawn_blocking(move || {
            super::remote_update::recover_remote_transaction(
                &window,
                &host,
                port.unwrap_or(22),
                &user,
                key_path.as_deref(),
                &install_dir,
            )
        })
        .await
        .map_err(|error| format!("恢复部署事务失败：{error}"))?;
    }
    tauri::async_runtime::spawn_blocking(move || recover_local(&window, &install_dir))
        .await
        .map_err(|error| format!("恢复部署事务失败：{error}"))?
}

fn progress(window: &tauri::Window, step: &str, percent: usize, status: &str, message: &str) {
    let _ = window.emit(
        "remote-update-progress",
        super::remote_update::RemoteUpdateProgress {
            step: step.to_string(),
            percent,
            status: status.to_string(),
            message: message.to_string(),
        },
    );
}

fn run_full_deploy(
    window: &tauri::Window,
    bundle_dir: &str,
    install_dir: &str,
    dry_run: bool,
) -> Result<VersionDeployResult, String> {
    let started = Instant::now();
    progress(
        window,
        "预检完整版本",
        3,
        "running",
        "核对交付包、数据卷与旧版本",
    );
    let install_root = resolve_install_root(install_dir, false)?;
    if dry_run && install_root.exists() {
        deployment_tx::ensure_no_incomplete_transaction(&install_root)?;
    }
    let _lock = if dry_run {
        None
    } else {
        if !install_root.exists() {
            fs::create_dir_all(&install_root)
                .map_err(|error| format!("无法创建部署目录 {}：{error}", install_root.display()))?;
        }
        let lock = DeploymentLock::acquire(&install_root)?;
        deployment_tx::ensure_no_incomplete_transaction(&install_root)?;
        Some(lock)
    };
    let plan = build_plan(bundle_dir, install_dir)?;
    let _project_lock = if dry_run || plan.project_name.trim().is_empty() {
        None
    } else {
        Some(DeploymentLock::acquire_project(&plan.project_name)?)
    };
    if !dry_run {
        validate_current_project_scope(&plan.install_root, &plan.deploy_dir, &plan.project_name)?;
    }
    if dry_run {
        let output = dry_run_output(&plan);
        progress(
            window,
            "预检通过",
            100,
            "success",
            "未停止服务，也未修改部署目录",
        );
        return Ok(result_without_transaction(
            &plan,
            true,
            false,
            PHASE_PREPARING,
            None,
            output,
            started.elapsed().as_secs(),
        ));
    }

    let mut transaction = DeploymentTransaction::new(
        &plan.install_root,
        &plan.deploy_dir,
        plan.bundle.product(),
        plan.bundle.version(),
        &plan.previous_version,
        plan.had_deployment,
        &plan.project_name,
        &plan.old_compose_rel,
        &plan.new_compose_rel,
    )?;
    transaction.old_images = plan.old_images.clone();
    transaction.services = plan.services.clone();
    transaction.previous_services = plan.old_services.clone();
    transaction.frontend_service = plan.frontend_service.clone();
    transaction.frontend_port = plan.frontend_port;
    transaction.set_phase(&plan.install_root, PHASE_PREPARING)?;

    let execution = execute_new_version(window, &plan, &mut transaction);
    match execution {
        Ok(output) => {
            progress(
                window,
                "部署完成",
                100,
                "success",
                "完整版本已启动并通过就绪检查",
            );
            Ok(VersionDeployResult {
                success: true,
                recovered: false,
                recovery_failed: false,
                stage: PHASE_COMMITTED.to_string(),
                version: plan.bundle.version().to_string(),
                previous_version: plan.previous_version.clone(),
                transaction_id: transaction.id.clone(),
                snapshot_dir: transaction.snapshot_dir.clone(),
                error: None,
                output,
                duration_secs: started.elapsed().as_secs(),
                status: Some(transaction),
            })
        }
        Err((stage, error)) => {
            let _ = transaction.fail(&plan.install_root, &error);
            log::error!(
                target: "deploy",
                "完整版本部署失败：目录={} 阶段={stage}，详情保留在部署事务中",
                plan.install_root.display()
            );
            progress(
                window,
                "新版本失败，开始恢复",
                88,
                "running",
                "正在恢复上一版本的文件、数据卷与容器",
            );
            let recovery = recover_from_transaction(&plan.install_root, &mut transaction);
            match recovery {
                Ok(()) => {
                    progress(
                        window,
                        "已恢复上一版本",
                        100,
                        "warning",
                        "新版本部署失败，上一版本已恢复运行",
                    );
                    Ok(VersionDeployResult {
                        success: false,
                        recovered: plan.had_deployment,
                        recovery_failed: false,
                        stage: stage.clone(),
                        version: plan.bundle.version().to_string(),
                        previous_version: plan.previous_version.clone(),
                        transaction_id: transaction.id.clone(),
                        snapshot_dir: transaction.snapshot_dir.clone(),
                        error: Some(error.clone()),
                        output: if plan.had_deployment {
                            format!(
                                "新版本部署失败，已恢复上一版本 {}\n失败阶段：{stage}\n原因：{error}",
                                if plan.previous_version.is_empty() {
                                    "未知版本"
                                } else {
                                    &plan.previous_version
                                }
                            )
                        } else {
                            format!(
                                "首次部署失败，已清理本次创建的服务与数据卷\n失败阶段：{stage}\n原因：{error}"
                            )
                        },
                        duration_secs: started.elapsed().as_secs(),
                        status: Some(transaction),
                    })
                }
                Err(recovery_error) => {
                    let _ = transaction.recovery_failed(&plan.install_root, &recovery_error);
                    progress(
                        window,
                        "自动恢复未完成",
                        100,
                        "error",
                        "部署现场已保留，请使用事务快照人工恢复",
                    );
                    Ok(VersionDeployResult {
                        success: false,
                        recovered: false,
                        recovery_failed: true,
                        stage: stage.clone(),
                        version: plan.bundle.version().to_string(),
                        previous_version: plan.previous_version.clone(),
                        transaction_id: transaction.id.clone(),
                        snapshot_dir: transaction.snapshot_dir.clone(),
                        error: Some(format!(
                            "{error}\n自动恢复失败：{recovery_error}\n快照目录：{}",
                            transaction.snapshot_dir
                        )),
                        output: format!(
                            "新版本部署失败，自动恢复未完成。现场已保留：{}\n失败阶段：{stage}\n恢复错误：{recovery_error}",
                            transaction.snapshot_dir
                        ),
                        duration_secs: started.elapsed().as_secs(),
                        status: Some(transaction),
                    })
                }
            }
        }
    }
}

fn execute_new_version(
    window: &tauri::Window,
    plan: &FullDeployPlan,
    transaction: &mut DeploymentTransaction,
) -> Result<String, (String, String)> {
    let mut output = String::new();
    transaction.deployment_started = true;
    deployment_tx::write_transaction(&plan.install_root, transaction)
        .map_err(|error| (PHASE_PREPARING.to_string(), error))?;
    if plan.had_deployment {
        progress(window, "停止旧版本", 7, "running", "停止全部应用与数据服务");
        transaction
            .set_phase(&plan.install_root, PHASE_SNAPSHOTTING_FILES)
            .map_err(|error| (PHASE_SNAPSHOTTING_FILES.to_string(), error))?;
        if let Some(old_compose) = &plan.old_compose {
            compose_checked(
                &plan.install_root,
                &plan.deploy_dir,
                old_compose,
                &["down", "--remove-orphans"],
                None,
                Duration::from_secs(600),
            )
            .map_err(|error| (PHASE_SNAPSHOTTING_FILES.to_string(), error))?;
        }
    }

    progress(
        window,
        "保存部署文件",
        14,
        "running",
        "建立可恢复的部署前文件快照",
    );
    deployment_tx::snapshot_managed_files(&plan.install_root, transaction, &plan.managed_paths)
        .map_err(|error| (PHASE_SNAPSHOTTING_FILES.to_string(), error))?;
    output.push_str(&format!(
        "文件快照：{} 项，目录 {}\n",
        transaction.files.len(),
        transaction.snapshot_dir
    ));

    progress(
        window,
        "备份数据卷",
        24,
        "running",
        "冷备数据库、消息队列与缓存数据",
    );
    transaction
        .set_phase(&plan.install_root, PHASE_SNAPSHOTTING_DATA)
        .map_err(|error| (PHASE_SNAPSHOTTING_DATA.to_string(), error))?;
    for volume in &plan.volumes {
        if !plan.had_deployment {
            transaction.volumes.push(VolumeSnapshot {
                logical_name: volume.logical_name.clone(),
                actual_name: volume.actual_name.clone(),
                image: volume.image.clone(),
                existed: false,
                file: String::new(),
                sha256: String::new(),
                bytes: 0,
            });
            deployment_tx::write_transaction(&plan.install_root, transaction)
                .map_err(|error| (PHASE_SNAPSHOTTING_DATA.to_string(), error))?;
            continue;
        }
        if !volume.exists {
            transaction.volumes.push(VolumeSnapshot {
                logical_name: volume.logical_name.clone(),
                actual_name: volume.actual_name.clone(),
                image: volume.image.clone(),
                existed: false,
                file: String::new(),
                sha256: String::new(),
                bytes: 0,
            });
            deployment_tx::write_transaction(&plan.install_root, transaction)
                .map_err(|error| (PHASE_SNAPSHOTTING_DATA.to_string(), error))?;
            continue;
        }
        progress(
            window,
            "备份数据卷",
            25,
            "running",
            &format!("正在备份 {}", volume.logical_name),
        );
        let destination = transaction
            .snapshot_path()
            .join("volumes")
            .join(format!("{}.tar.gz", volume.logical_name));
        let (sha256, bytes) =
            deployment_tx::backup_volume_local(&volume.image, &volume.actual_name, &destination)
                .map_err(|error| (PHASE_SNAPSHOTTING_DATA.to_string(), error))?;
        transaction.volumes.push(VolumeSnapshot {
            logical_name: volume.logical_name.clone(),
            actual_name: volume.actual_name.clone(),
            image: volume.image.clone(),
            existed: true,
            file: destination.to_string_lossy().to_string(),
            sha256,
            bytes,
        });
        deployment_tx::write_transaction(&plan.install_root, transaction)
            .map_err(|error| (PHASE_SNAPSHOTTING_DATA.to_string(), error))?;
        output.push_str(&format!("数据卷 {}：{} 字节\n", volume.logical_name, bytes));
    }
    transaction
        .set_phase(&plan.install_root, PHASE_APPLYING)
        .map_err(|error| (PHASE_APPLYING.to_string(), error))?;

    progress(
        window,
        "写入完整版本",
        55,
        "running",
        "写入全部交付文件与环境变量",
    );
    let resolved_images = image_snapshots(&plan.install_root, &plan.incoming_config)
        .map_err(|error| (PHASE_APPLYING.to_string(), error))?;
    fs::create_dir_all(&plan.deploy_dir).map_err(|error| {
        (
            PHASE_APPLYING.to_string(),
            format!("创建部署目录失败：{error}"),
        )
    })?;
    if !plan.retired_paths.is_empty() {
        let retired: Vec<PathBuf> = plan
            .retired_paths
            .iter()
            .map(|relative| plan.install_root.join(relative))
            .collect();
        deployment_tx::remove_managed_files(&retired)
            .map_err(|error| (PHASE_APPLYING.to_string(), error))?;
    }
    for artifact in &plan
        .bundle
        .resolve_artifacts(
            &plan
                .bundle
                .select_components(&plan.components)
                .map_err(|error| (PHASE_APPLYING.to_string(), error))?,
            &plan.install_root,
            &plan.deploy_dir,
        )
        .map_err(|error| (PHASE_APPLYING.to_string(), error))?
    {
        replace_path(&artifact.source, &artifact.target)
            .map_err(|error| (PHASE_APPLYING.to_string(), error))?;
        transaction.applied_paths.push(
            deployment_tx::relative_install_path(&plan.install_root, &artifact.target)
                .map_err(|error| (PHASE_APPLYING.to_string(), error))?,
        );
        deployment_tx::write_transaction(&plan.install_root, transaction)
            .map_err(|error| (PHASE_APPLYING.to_string(), error))?;
        output.push_str(&format!("已写入：{}\n", artifact.target.to_string_lossy()));
    }
    if let Some(content) = &plan.env_content {
        super::projects::write_private_env(&plan.deploy_dir.join(".env"), content)
            .map_err(|error| (PHASE_APPLYING.to_string(), error))?;
    }
    let effective = compose_config(
        &plan.install_root,
        &plan.deploy_dir,
        &plan.new_compose,
        false,
    )
    .map_err(|error| (PHASE_APPLYING.to_string(), error))?;
    if effective.get("name").and_then(Value::as_str) != Some(plan.project_name.as_str()) {
        return Err((
            PHASE_APPLYING.to_string(),
            "新版本 Compose 项目名称与部署前不一致，已停止启动".to_string(),
        ));
    }

    progress(
        window,
        "启动新版本",
        76,
        "running",
        "启动完整服务栈并等待健康检查",
    );
    transaction
        .set_phase(&plan.install_root, PHASE_STARTING)
        .map_err(|error| (PHASE_STARTING.to_string(), error))?;
    transaction.startup_attempted = true;
    deployment_tx::write_transaction(&plan.install_root, transaction)
        .map_err(|error| (PHASE_STARTING.to_string(), error))?;
    let override_document = image_override_document(
        &plan.incoming_config,
        &resolved_images.values().cloned().collect::<Vec<_>>(),
    )
    .map_err(|error| (PHASE_STARTING.to_string(), error))?;
    compose_checked(
        &plan.install_root,
        &plan.deploy_dir,
        &plan.new_compose,
        &["up", "-d", "--remove-orphans"],
        override_document.as_deref(),
        Duration::from_secs(1800),
    )
    .map_err(|error| (PHASE_STARTING.to_string(), error))?;
    wait_for_services(
        &plan.install_root,
        &plan.deploy_dir,
        &plan.new_compose,
        &plan.services,
        frontend_probe(&plan.frontend_service, plan.frontend_port),
    )
    .map_err(|error| (PHASE_STARTING.to_string(), error))?;

    let record = deployed_record_v2(
        &plan.bundle,
        &plan.install_root,
        &plan.deploy_dir,
        &plan.project_id,
        &resolved_images,
        &plan.volumes,
        &transaction.id,
    )
    .map_err(|error| (PHASE_STARTING.to_string(), error))?;
    write_deployed_record(&plan.install_root, &record)
        .map_err(|error| (PHASE_STARTING.to_string(), error))?;
    transaction.phase = PHASE_COMMITTED.to_string();
    transaction.state = "committed".to_string();
    transaction.updated_at = chrono::Utc::now().to_rfc3339();
    deployment_tx::write_transaction(&plan.install_root, transaction)
        .map_err(|error| (PHASE_STARTING.to_string(), error))?;
    let _ = fs::remove_dir_all(transaction.snapshot_path());
    output.push_str(&format!(
        "部署成功：版本 {}，事务 {}",
        plan.bundle.version(),
        transaction.id
    ));
    Ok(output)
}

fn project_id() -> String {
    super::projects::active_project()
        .map(|project| project.id)
        .unwrap_or_default()
}

fn recover_local(window: &tauri::Window, install_dir: &str) -> Result<VersionDeployResult, String> {
    let started = Instant::now();
    let install = resolve_install_root(install_dir, true)?;
    let _lock = DeploymentLock::acquire(&install)?;
    let mut transaction = deployment_tx::incomplete_transaction(&install)?
        .ok_or_else(|| "未找到需要恢复的部署事务".to_string())?;
    let _project_lock = if transaction.project_name.trim().is_empty() {
        None
    } else {
        Some(DeploymentLock::acquire_project(&transaction.project_name)?)
    };
    if !transaction.project_name.trim().is_empty() {
        validate_current_project_scope(
            &install,
            Path::new(&transaction.deploy_dir),
            &transaction.project_name,
        )?;
    }
    if !matches!(
        transaction.state.as_str(),
        "pending" | "running" | "failed" | "recovering" | "recovery_failed"
    ) {
        return Err("部署事务状态无法识别，请保留现场并人工检查".to_string());
    }
    if transaction.state == "recovered" {
        return Err("该部署事务已经恢复，请勿重复执行".to_string());
    }
    if transaction.state == "recovery_failed" {
        return Err("该部署事务上次恢复失败，请按快照目录人工处理，系统不会自动重试".to_string());
    }
    if transaction.state == "recovering" {
        return Err("该部署事务正在恢复或上次恢复被中断，请先检查事务现场后再处理".to_string());
    }
    progress(
        window,
        "恢复上一版本",
        10,
        "running",
        "停止失败版本并读取事务快照",
    );
    let recovery = recover_from_transaction(&install, &mut transaction);
    let duration_secs = started.elapsed().as_secs();
    match recovery {
        Ok(()) => {
            let recovered = transaction.had_deployment && transaction.deployment_started;
            Ok(VersionDeployResult {
                success: false,
                recovered,
                recovery_failed: false,
                stage: PHASE_RECOVERING.to_string(),
                version: transaction.version.clone(),
                previous_version: transaction.previous_version.clone(),
                transaction_id: transaction.id.clone(),
                snapshot_dir: transaction.snapshot_dir.clone(),
                error: transaction.error.clone(),
                output: if !transaction.deployment_started {
                    format!(
                        "本次事务在修改目标前已停止，无需恢复服务、文件或数据卷\n事务：{}",
                        transaction.id
                    )
                } else if recovered {
                    format!(
                        "已恢复上一版本 {}\n事务：{}",
                        if transaction.previous_version.is_empty() {
                            "未知版本"
                        } else {
                            &transaction.previous_version
                        },
                        transaction.id
                    )
                } else {
                    format!(
                        "首次部署事务已清理，未保留上一版本\n事务：{}",
                        transaction.id
                    )
                },
                duration_secs,
                status: Some(transaction),
            })
        }
        Err(error) => {
            let _ = transaction.recovery_failed(&install, &error);
            Ok(VersionDeployResult {
                success: false,
                recovered: false,
                recovery_failed: true,
                stage: PHASE_RECOVERING.to_string(),
                version: transaction.version.clone(),
                previous_version: transaction.previous_version.clone(),
                transaction_id: transaction.id.clone(),
                snapshot_dir: transaction.snapshot_dir.clone(),
                error: Some(error.clone()),
                output: format!(
                    "恢复未完成，现场已保留：{}\n原因：{error}",
                    transaction.snapshot_dir
                ),
                duration_secs,
                status: Some(transaction),
            })
        }
    }
}

fn recover_from_transaction(
    install: &Path,
    transaction: &mut DeploymentTransaction,
) -> Result<(), String> {
    let deploy_dir = PathBuf::from(&transaction.deploy_dir);
    let new_compose = transaction_compose_path(install, &transaction.new_compose);
    let mut previous = None;
    if transaction.deployment_started {
        deployment_tx::verify_recovery_snapshot(transaction)?;
        if transaction.startup_attempted && !new_compose.is_file() {
            return Err(
                "失败版本的 Compose 清单缺失，无法确认服务已停止；请检查后人工恢复".to_string(),
            );
        }
        for image in &transaction.old_images {
            if !super::deployment_image_bindings::valid_image_id(&image.image_id) {
                return Err(format!(
                    "上一版本部署记录中的镜像 {} 标识无效，无法安全恢复",
                    image.reference
                ));
            }
            if inspect_image(&image.image_id)?.is_none() {
                return Err(format!("上一版本镜像 {} 已不存在，无法安全恢复", image.reference));
            }
        }
        if transaction.startup_attempted {
            for volume in transaction.volumes.iter().filter(|volume| volume.existed) {
                if !super::deployment_image_bindings::valid_image_id(&volume.image) {
                    return Err(format!(
                        "数据卷 {} 的备份镜像标识无效，无法安全恢复",
                        volume.logical_name
                    ));
                }
                if inspect_image(&volume.image)?.is_none() {
                    return Err(format!(
                        "数据卷 {} 的备份镜像已不存在，无法安全恢复",
                        volume.logical_name
                    ));
                }
                deployment_tx::check_backup_tools_local(&volume.image)?;
            }
        }
        if transaction.had_deployment && !transaction.previous_compose.is_empty() {
            let old_compose = transaction_compose_path(install, &transaction.previous_compose);
            if !old_compose.is_file() {
                return Err(format!(
                    "上一版本 Compose 清单缺失，无法安全恢复：{}",
                    old_compose.display()
                ));
            }
            let old_config = compose_config(install, &deploy_dir, &old_compose, false)?;
            let old_override =
                image_override_document(&old_config, &transaction.old_images)?;
            previous = Some((old_compose, old_override, old_config));
        }
    }
    transaction.recover(install)?;
    if !transaction.deployment_started {
        transaction.recovered(install)?;
        return Ok(());
    }
    if transaction.startup_attempted {
        compose_checked(
            install,
            &deploy_dir,
            &new_compose,
            &["down", "--remove-orphans"],
            None,
            Duration::from_secs(600),
        )
        .map_err(|error| format!("停止失败版本失败，未写入恢复内容：{error}"))?;
    }
    deployment_tx::restore_managed_files(install, transaction)?;
    for volume in transaction
        .volumes
        .iter()
        .filter(|_| transaction.startup_attempted)
    {
        if !volume.existed {
            remove_volume_if_present(&volume.actual_name)?;
            continue;
        }
        deployment_tx::restore_volume_local(
            &volume.image,
            &volume.actual_name,
            Path::new(&volume.file),
        )?;
    }
    if let Some((old_compose, old_override, old_config)) = previous {
        compose_checked(
            install,
            &deploy_dir,
            &old_compose,
            &["up", "-d", "--remove-orphans"],
            old_override.as_deref(),
            Duration::from_secs(1800),
        )?;
        let services = if transaction.previous_services.is_empty() {
            let config = compose_config(install, &deploy_dir, &old_compose, false)?;
            active_services(&config)
        } else {
            transaction.previous_services.clone()
        };
        if !services.is_empty() {
            wait_for_services(
                install,
                &deploy_dir,
                &old_compose,
                &services,
                frontend_probe(
                    &transaction.frontend_service,
                    published_frontend_port(
                        &old_config,
                        &transaction.frontend_service,
                        transaction.frontend_port,
                    ),
                ),
            )?;
        }
    }
    transaction.recovered(install)?;
    Ok(())
}

fn remove_volume_if_present(actual_name: &str) -> Result<(), String> {
    let exists = docker_checked(
        &["volume", "inspect", actual_name],
        "检查待清理的数据卷",
        Duration::from_secs(30),
    )
    .is_ok();
    if !exists {
        return Ok(());
    }
    docker_checked(
        &["volume", "rm", actual_name],
        "清理部署前不存在的数据卷",
        Duration::from_secs(120),
    )
    .map(|_| ())
}

pub(crate) fn image_override_document(
    config: &Value,
    images: &[ImageSnapshot],
) -> Result<Option<String>, String> {
    let Some(services) = config.get("services").and_then(Value::as_object) else {
        return Err("部署清单缺少服务，无法固定恢复镜像".to_string());
    };
    let mut overrides = BTreeMap::new();
    for (name, service) in services {
        if !active_services(config).contains(name) {
            continue;
        }
        let reference = service
            .get("image")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let image = images
            .iter()
            .find(|image| image.reference == reference)
            .filter(|image| super::deployment_image_bindings::valid_image_id(&image.image_id))
            .ok_or_else(|| format!("服务 {name} 缺少部署前核对的镜像 ID，无法安全启动"))?;
        overrides.insert(
            name,
            serde_json::json!({ "image": image.image_id, "pull_policy": "never" }),
        );
    }
    serde_json::to_string(&serde_json::json!({ "services": overrides }))
        .map(Some)
        .map_err(|error| format!("无法固定部署镜像：{error}"))
}

pub(crate) fn validate_new_deployment_isolation(
    config: &Value,
    has_project_containers: bool,
) -> Result<(), String> {
    if has_project_containers {
        return Ok(());
    }
    let Some(services) = config.get("services").and_then(Value::as_object) else {
        return Ok(());
    };
    for (name, service) in services {
        if service
            .get("container_name")
            .and_then(Value::as_str)
            .is_some()
        {
            return Err(format!(
                "首次部署的服务 {name} 使用固定容器名，无法确认是否会影响已有部署"
            ));
        }
        for port in service
            .get("ports")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if !port.get("host_ip").is_some_and(|value| {
                value
                    .as_str()
                    .is_some_and(|text| !text.is_empty() && text != "127.0.0.1")
            }) {
                return Err(format!(
                    "首次部署的服务 {name} 使用可能冲突的宿主端口，请先隔离旧服务或改用专用端口"
                ));
            }
        }
        for mount in service
            .get("volumes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if mount.get("type").and_then(Value::as_str) == Some("volume")
                && mount.get("source").is_none()
            {
                return Err(format!(
                    "首次部署的服务 {name} 包含匿名卷，部署失败后无法可靠清理"
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_project_container_scope(
    project_name: &str,
    install: &Path,
    deploy_dir: &Path,
    raw: &str,
) -> Result<bool, String> {
    let mut found = false;
    for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        found = true;
        let working_dir = line
            .split_once('|')
            .map(|(working, _)| working.trim())
            .unwrap_or(line);
        if working_dir.is_empty() {
            return Err(format!(
                "Compose 项目 {project_name} 的容器缺少工作目录标签，无法确认部署边界"
            ));
        }
        if !path_guard::path_is_within(deploy_dir, Path::new(working_dir)) {
            return Err(format!(
                "Compose 项目 {project_name} 已被其他部署目录使用（{working_dir}），已阻止部署"
            ));
        }
        if !path_guard::path_is_within(install, Path::new(working_dir)) {
            return Err(format!(
                "Compose 项目 {project_name} 的工作目录不在当前部署目录内，已阻止部署"
            ));
        }
    }
    Ok(found)
}

fn validate_current_project_scope(
    install: &Path,
    deploy_dir: &Path,
    project_name: &str,
) -> Result<(), String> {
    let raw = docker_checked(
        &[
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={project_name}"),
            "--format",
            "{{.Label \"com.docker.compose.project.working_dir\"}}|{{.Label \"com.docker.compose.project.config_files\"}}",
        ],
        "检查 Compose 项目容器边界",
        Duration::from_secs(60),
    )?;
    validate_project_container_scope(project_name, install, deploy_dir, &raw)?;
    Ok(())
}

pub(crate) fn published_frontend_port(
    config: &Value,
    service: &str,
    preferred: Option<u16>,
) -> Option<u16> {
    let service = config.get("services")?.get(service)?;
    let ports: Vec<u16> = service
        .get("ports")?
        .as_array()?
        .iter()
        .filter_map(|port| {
            if port
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("tcp")
                != "tcp"
            {
                return None;
            }
            let published = port.get("published")?;
            published
                .as_str()
                .and_then(|value| value.parse().ok())
                .or_else(|| {
                    published
                        .as_u64()
                        .and_then(|value| u16::try_from(value).ok())
                })
        })
        .collect();
    preferred
        .filter(|preferred| ports.contains(preferred))
        .or_else(|| (ports.len() == 1).then(|| ports[0]))
}

fn transaction_compose_path(install: &Path, stored: &str) -> PathBuf {
    install.join(stored)
}

fn build_plan(bundle_dir: &str, install_dir: &str) -> Result<FullDeployPlan, String> {
    let bundle_root =
        fs::canonicalize(bundle_dir.trim()).map_err(|_| "无法访问交付包目录".to_string())?;
    let install_root = resolve_install_root(install_dir, false)?;
    if bundle_root.starts_with(&install_root) || install_root.starts_with(&bundle_root) {
        return Err("交付包与部署目录不能重叠".to_string());
    }
    let bundle = Bundle::load(&bundle_root)?;
    bundle.ensure_compatible()?;
    bundle.verify_payload()?;
    bundle.ensure_full_deploy_metadata()?;
    check_local_docker()?;

    let deploy_dir = install_root.join(bundle.deploy_source_rel());
    let components = default_components(&bundle);
    let selected = bundle.select_components(&components)?;
    let artifacts = bundle.resolve_artifacts(&selected, &install_root, &deploy_dir)?;
    if artifacts.is_empty() {
        return Err("交付包未声明可部署产物".to_string());
    }
    for artifact in &artifacts {
        if !artifact.source.exists() {
            return Err(format!("交付包缺少产物文件：{}", artifact.label));
        }
        ensure_no_symlink(&artifact.source)?;
    }
    verify_service_declarations(&bundle)?;

    let compose_source_rel = bundle.effective_compose().to_string();
    let compose_artifact = bundle
        .artifact_for_source(&compose_source_rel)
        .ok_or_else(|| "交付包未声明 Compose 清单".to_string())?;
    let compose_base = match compose_artifact.root {
        ArtifactRoot::Install => install_root.clone(),
        ArtifactRoot::Deploy => deploy_dir.clone(),
    };
    let new_compose = compose_base.join(compose_artifact.path.trim());
    let new_compose_rel = deployment_tx::relative_install_path(&install_root, &new_compose)?;
    let new_compose_source = bundle.root.join(&compose_source_rel);
    ensure_no_symlink(&new_compose_source)?;

    let record = super::bundle::read_deployed_record_strict(&install_root)?;
    if record.as_ref().is_some_and(|record| {
        record.product != bundle.product() || !matches!(record.schema_version, 1 | 2)
    }) {
        return Err("目标目录的旧部署记录与当前产品或记录版本不匹配，已停止部署".to_string());
    }
    let old_compose = find_old_compose(&install_root, &deploy_dir, record.as_ref())?;
    let old_compose_rel = old_compose
        .as_ref()
        .map(|path| deployment_tx::relative_install_path(&install_root, path))
        .transpose()?
        .unwrap_or_default();
    let had_deployment = old_compose.is_some();
    let previous_version = record
        .as_ref()
        .map(|item| item.version.clone())
        .unwrap_or_default();

    let env_content = super::projects::read_active_project_env()?;
    let deployment_project_id = project_id();
    let incoming_config =
        compose_config_bundle(&new_compose_source, &deploy_dir, env_content.as_deref())?;
    let project_name = compose_project_name(&incoming_config)
        .ok_or_else(|| "新版本 Compose 清单缺少项目名称".to_string())?;
    let project_container_scope = docker_checked(
        &[
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={project_name}"),
            "--format",
            "{{.Label \"com.docker.compose.project.working_dir\"}}|{{.Label \"com.docker.compose.project.config_files\"}}",
        ],
        "检查 Compose 项目容器边界",
        Duration::from_secs(60),
    )?;
    let project_containers = validate_project_container_scope(
        &project_name,
        &install_root,
        &deploy_dir,
        &project_container_scope,
    )?;
    if !had_deployment && project_containers {
        return Err(format!(
            "Compose 项目 {project_name} 已有容器，但当前目录没有可恢复的部署记录，已阻止部署"
        ));
    }
    if !had_deployment {
        validate_new_deployment_isolation(&incoming_config, false)?;
    }
    let current_config = if let Some(compose) = &old_compose {
        let current = compose_config(&install_root, &deploy_dir, compose, false)?;
        let current_name = compose_project_name(&current)
            .ok_or_else(|| "当前 Compose 项目名称无法解析".to_string())?;
        if current_name != project_name {
            return Err(format!(
                "新旧版本 Compose 项目名称不同（{} -> {}），会指向另一组数据卷，已阻止部署",
                current_name, project_name
            ));
        }
        Some(current)
    } else {
        None
    };

    let services = active_services(&incoming_config);
    let old_services = current_config
        .as_ref()
        .map(active_services)
        .unwrap_or_default();
    verify_all_declared_services(&bundle, &services)?;
    verify_declared_image_references(&bundle, &incoming_config)?;
    image_snapshots(&install_root, &incoming_config)?;
    let old_images = current_config
        .as_ref()
        .map(|config| previous_image_snapshots(&install_root, config, record.as_ref()))
        .transpose()?
        .unwrap_or_default();
    let volumes = volume_plans(
        &install_root,
        &bundle,
        &incoming_config,
        current_config.as_ref(),
        had_deployment,
    )?;
    let mut managed_paths: BTreeSet<String> = artifacts
        .iter()
        .map(|artifact| deployment_tx::relative_install_path(&install_root, &artifact.target))
        .collect::<Result<_, _>>()?;
    let mut new_paths = managed_paths.clone();
    let mut retired_paths = BTreeSet::new();
    if let Some(record) = &record {
        for artifact in &record.artifacts {
            let relative = match artifact.root.as_str() {
                "install" => artifact.path.clone(),
                "deploy" => {
                    if bundle.deploy_source_rel().is_empty() {
                        artifact.path.clone()
                    } else {
                        format!("{}/{}", bundle.deploy_source_rel(), artifact.path)
                    }
                }
                _ => continue,
            };
            deployment_tx::validate_artifact_relative(&relative)?;
            managed_paths.insert(relative.clone());
            if !new_paths.contains(&relative) {
                retired_paths.insert(relative);
            }
        }
    }
    let env_rel = deployment_tx::relative_install_path(&install_root, &deploy_dir.join(".env"))?;
    managed_paths.insert(env_rel.clone());
    new_paths.insert(env_rel);
    let deployed_rel = format!("{DEPLOYED_RECORD_DIR}/{DEPLOYED_RECORD_FILE}");
    managed_paths.insert(deployed_rel.clone());
    new_paths.insert(deployed_rel);
    if !old_compose_rel.is_empty() {
        managed_paths.insert(old_compose_rel.clone());
        if old_compose_rel != new_compose_rel {
            retired_paths.insert(old_compose_rel.clone());
        }
    }
    managed_paths.insert(new_compose_rel.clone());
    new_paths.insert(new_compose_rel.clone());
    retired_paths.retain(|path| !new_paths.contains(path));
    let retired_paths = minimal_retired_paths(retired_paths);
    validate_deployment_mounts(
        &bundle,
        &install_root.to_string_lossy(),
        &managed_paths,
        &incoming_config,
    )?;
    if let Some(current) = &current_config {
        validate_deployment_mounts(
            &bundle,
            &install_root.to_string_lossy(),
            &managed_paths,
            current,
        )?;
    }
    ensure_preserved_paths_untouched(&managed_paths, &bundle)?;
    ensure_preserved_paths_untouched(&retired_paths.iter().cloned().collect(), &bundle)?;

    let mut file_bytes = 0u64;
    for relative in &managed_paths {
        deployment_tx::validate_managed_relative(relative)?;
        let target = install_root.join(relative);
        for ancestor in target
            .ancestors()
            .take_while(|path| path.starts_with(&install_root))
        {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err("目标受管路径包含符号链接，无法安全部署".to_string())
                }
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(format!("无法读取目标部署路径：{error}"))
                }
                _ => {}
            }
        }
        file_bytes = file_bytes.saturating_add(tree_bytes(&target)?);
    }
    let volume_bytes = volumes.iter().map(|volume| volume.bytes).sum::<u64>();
    let incoming_bytes = artifacts
        .iter()
        .try_fold(0u64, |total, artifact| {
            tree_bytes(&artifact.source).map(|bytes| total.saturating_add(bytes))
        })?;
    let image_bytes = bundle
        .image_files()
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
    let required_backup_bytes = file_bytes
        .saturating_add(incoming_bytes)
        .saturating_add(volume_bytes)
        .saturating_add(image_bytes)
        .saturating_add(512 * 1024 * 1024);
    let free = available_disk_bytes(&install_root)?;
    if free < required_backup_bytes {
        return Err(format!(
            "部署分区空间不足：需要约 {:.1} GB，可用 {:.1} GB",
            required_backup_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
            free as f64 / 1024.0 / 1024.0 / 1024.0
        ));
    }

    let (frontend_service, frontend_port) = frontend_declaration(&bundle);
    if let Some(current) = &current_config {
        if current
            .get("services")
            .and_then(|services| services.get(&frontend_service))
            .is_some()
            && published_frontend_port(current, &frontend_service, frontend_port).is_none()
        {
            return Err(
                "无法确定上一版本网页的 HTTP 端口，请确认旧版本的网页端口声明后再部署".to_string(),
            );
        }
    }
    Ok(FullDeployPlan {
        install_root,
        deploy_dir,
        bundle,
        components,
        managed_paths: managed_paths.into_iter().collect(),
        retired_paths,
        new_compose,
        new_compose_rel,
        old_compose,
        old_compose_rel,
        had_deployment,
        previous_version,
        project_name,
        incoming_config,
        old_images,
        volumes,
        services,
        old_services,
        frontend_service,
        frontend_port,
        required_backup_bytes,
        env_content,
        project_id: deployment_project_id,
    })
}

fn frontend_declaration(bundle: &Bundle) -> (String, Option<u16>) {
    bundle
        .descriptor
        .runtime
        .services
        .iter()
        .find(|service| service.key.eq_ignore_ascii_case("frontend"))
        .map(|service| (service.key.clone(), service.port))
        .unwrap_or_default()
}

fn dry_run_output(plan: &FullDeployPlan) -> String {
    let data: Vec<String> = plan
        .volumes
        .iter()
        .map(|volume| format!("{}（{}）", volume.logical_name, volume.actual_name))
        .collect();
    format!(
        "预检通过：版本 {}；部署目录 {}；{} 个服务；数据卷：{}；预计备份空间约 {:.1} GB。未停止服务，也未修改目标。",
        plan.bundle.version(),
        plan.install_root.display(),
        plan.services.len(),
        if data.is_empty() {
            "无".to_string()
        } else {
            data.join("、")
        },
        plan.required_backup_bytes as f64 / 1024.0 / 1024.0 / 1024.0
    )
}

fn result_without_transaction(
    plan: &FullDeployPlan,
    success: bool,
    recovered: bool,
    stage: &str,
    error: Option<String>,
    output: String,
    duration_secs: u64,
) -> VersionDeployResult {
    VersionDeployResult {
        success,
        recovered,
        recovery_failed: false,
        stage: stage.to_string(),
        version: plan.bundle.version().to_string(),
        previous_version: plan.previous_version.clone(),
        transaction_id: String::new(),
        snapshot_dir: String::new(),
        error,
        output,
        duration_secs,
        status: None,
    }
}

fn volume_plans(
    install: &Path,
    bundle: &Bundle,
    incoming: &Value,
    current: Option<&Value>,
    had_deployment: bool,
) -> Result<Vec<VolumePlan>, String> {
    let data = bundle
        .descriptor
        .runtime
        .data
        .as_ref()
        .ok_or_else(|| "交付包缺少数据卷声明".to_string())?;
    let mut result = Vec::new();
    let bindings = super::deployment_image_bindings::local_bindings(install)?;
    let mut verified_backup_images = BTreeSet::new();
    for volume in &data.volumes {
        let actual = resolved_volume_name(incoming, &volume.name)?;
        ensure_volume_mounted_by(incoming, &volume.service, &volume.name)?;
        let reference = bundle
            .descriptor
            .images
            .iter()
            .find(|image| image.name.trim() == volume.image.trim())
            .map(|image| image.reference.clone())
            .ok_or_else(|| format!("数据卷 {} 的备份镜像未声明", volume.name))?;
        let identity = bindings
            .iter()
            .find(|binding| binding.reference == reference)
            .map(|binding| binding.image_id.as_str())
            .unwrap_or(&reference);
        let image = inspect_image(identity)?
            .ok_or_else(|| format!("目标 Docker 缺少数据卷备份镜像 {reference}"))?
            .image_id;
        if verified_backup_images.insert(image.clone()) {
            if inspect_image(&image)?.is_none() {
                return Err(format!(
                    "目标 Docker 缺少数据卷备份镜像 {image}，请先准备镜像后再部署"
                ));
            }
            deployment_tx::check_backup_tools_local(&image)?;
        }
        let exists = docker_checked(
            &["volume", "inspect", "--", &actual],
            "检查数据卷",
            Duration::from_secs(30),
        )
        .is_ok();
        if !had_deployment && exists {
            return Err(format!(
                "首次部署的目标已存在数据卷 {}，为避免误删已有数据，已停止部署",
                actual
            ));
        }
        if had_deployment && volume.required && !exists {
            return Err(format!(
                "现有部署缺少必需数据卷 {}，无法建立可恢复快照",
                actual
            ));
        }
        if exists {
            let owner = docker_checked(
                &["volume", "inspect", "--format", "{{index .Labels \"com.docker.compose.project\"}}|{{index .Labels \"com.docker.compose.volume\"}}", &actual],
                "检查数据卷归属", Duration::from_secs(30),
            )?;
            let expected = format!(
                "{}|{}",
                compose_project_name(incoming).unwrap_or_default(),
                volume.name
            );
            if owner.trim() != expected {
                return Err(format!(
                    "数据卷 {actual} 的 Compose 项目或逻辑卷标签不一致，已阻止部署"
                ));
            }
        }
        if let Some(current) = current {
            if resolved_volume_name(current, &volume.name)? != actual {
                return Err(format!(
                    "新旧版本的数据卷 {} 实际名称不一致，已停止部署",
                    volume.name
                ));
            }
        }
        let bytes = if exists {
            volume_usage_bytes(&image, &actual).unwrap_or(0)
        } else {
            0
        };
        result.push(VolumePlan {
            logical_name: volume.name.clone(),
            image,
            actual_name: actual,
            exists,
            bytes,
        });
    }
    for service in active_service_configs(incoming) {
        if let Some(items) = service.get("volumes").and_then(Value::as_array) {
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("volume") {
                    let source = item
                        .get("source")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !result.iter().any(|volume| volume.logical_name == source) {
                        return Err(format!(
                            "服务使用了未声明的数据卷 {source}，无法保证部署失败后恢复数据"
                        ));
                    }
                }
            }
        }
    }
    Ok(result)
}

fn resolved_volume_name(config: &Value, logical: &str) -> Result<String, String> {
    let volume = config
        .get("volumes")
        .and_then(|value| value.get(logical))
        .and_then(Value::as_object)
        .ok_or_else(|| format!("Compose 未声明数据卷 {logical}"))?;
    if volume.get("external").and_then(Value::as_bool) == Some(true) {
        return Err(format!("数据卷 {logical} 为 external，无法安全备份与恢复"));
    }
    volume
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("无法解析数据卷 {logical} 的实际名称"))
}

pub(crate) fn validate_deployment_mounts(
    bundle: &Bundle,
    install: &str,
    managed: &BTreeSet<String>,
    config: &Value,
) -> Result<(), String> {
    let data = bundle
        .descriptor
        .runtime
        .data
        .as_ref()
        .ok_or_else(|| "交付包缺少持久数据声明".to_string())?;
    let volumes = data
        .volumes
        .iter()
        .map(|volume| volume.name.clone())
        .collect::<Vec<_>>();
    let data_services = bundle
        .descriptor
        .runtime
        .services
        .iter()
        .filter(|service| service.role == Some(super::bundle::ServiceRole::Data))
        .map(|service| service.key.clone())
        .collect::<Vec<_>>();
    let preserve = data
        .paths
        .iter()
        .map(|path| format!("{install}/{}", path.path))
        .collect::<Vec<_>>();
    let managed = managed
        .iter()
        .map(|path| format!("{install}/{path}"))
        .collect::<Vec<_>>();
    validate_mount_contract(config, &volumes, &data_services, &preserve, &managed)
}

pub(crate) fn validate_mount_contract(
    config: &Value,
    volumes: &[String],
    data_services: &[String],
    preserve: &[String],
    managed: &[String],
) -> Result<(), String> {
    let Some(services) = config.get("services").and_then(Value::as_object) else {
        return Err("Compose 缺少服务清单".to_string());
    };
    for (name, service) in services {
        if service
            .get("volumes_from")
            .and_then(Value::as_array)
            .is_some_and(|sources| !sources.is_empty())
        {
            return Err(format!("服务 {name} 使用继承数据卷，无法确认数据保护范围"));
        }
        for mount in service
            .get("volumes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let source = mount
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match mount
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "volume" => {
                    if source.is_empty() || !volumes.iter().any(|volume| volume == source) {
                        return Err(format!(
                            "服务 {name} 使用匿名卷或未声明数据卷 {source}，无法安全恢复"
                        ));
                    }
                    resolved_volume_name(config, source)?;
                    if config
                        .pointer(&format!("/volumes/{source}/driver_opts"))
                        .and_then(Value::as_object)
                        .is_some_and(|options| !options.is_empty())
                    {
                        return Err(format!(
                            "数据卷 {source} 使用外部存储选项，本期无法自动恢复"
                        ));
                    }
                }
                "bind" => {
                    let read_only = mount
                        .get("read_only")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let preserved = preserve
                        .iter()
                        .any(|path| mount_path_contains(path, source));
                    let snapshotted = managed.iter().any(|path| mount_path_contains(path, source));
                    if (read_only && (preserved || snapshotted))
                        || (preserved && !snapshotted && !data_services.contains(name))
                    {
                        continue;
                    }
                    return Err(format!("服务 {name} 的宿主挂载 {source} 未纳入可恢复数据范围，请改用声明的数据卷或只读受管文件"));
                }
                "tmpfs" => {}
                kind => return Err(format!("服务 {name} 使用不支持的挂载类型 {kind}")),
            }
        }
    }
    Ok(())
}

fn mount_path_contains(root: &str, target: &str) -> bool {
    let normalize = |value: &str| {
        let value = path_guard::docker_path(Path::new(value))
            .to_string_lossy()
            .replace('\\', "/");
        let windows = value.as_bytes().get(1) == Some(&b':');
        let mut parts = Vec::new();
        for part in value.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                part => parts.push(part.to_string()),
            }
        }
        let normalized = parts.join("/");
        if windows {
            normalized.to_lowercase()
        } else {
            normalized
        }
    };
    let root = normalize(root);
    let target = normalize(target);
    !root.is_empty() && (root == target || target.starts_with(&format!("{root}/")))
}

fn ensure_volume_mounted_by(config: &Value, service: &str, logical: &str) -> Result<(), String> {
    let mounted = config
        .get("services")
        .and_then(|value| value.get(service))
        .and_then(|value| value.get("volumes"))
        .and_then(Value::as_array)
        .map(|volumes| {
            volumes.iter().any(|volume| {
                volume.get("type").and_then(Value::as_str) == Some("volume")
                    && volume.get("source").and_then(Value::as_str) == Some(logical)
            })
        })
        .unwrap_or(false);
    if mounted {
        Ok(())
    } else {
        Err(format!(
            "数据卷 {logical} 未挂载到声明的服务 {service}，请修正交付包"
        ))
    }
}

fn volume_usage_bytes(image: &str, actual: &str) -> Result<u64, String> {
    let mut command = crate::commands::docker_setup::docker_command();
    command
        .args(["run", "--rm", "--network", "none", "-v"])
        .arg(format!("{actual}:/data:ro"))
        .arg("--entrypoint")
        .arg("/bin/sh")
        .arg(image)
        .args(["-c", "du -sk /data 2>/dev/null | awk '{print $1}'"])
        .stdin(Stdio::null());
    let output = command
        .output()
        .map_err(|error| format!("估算数据卷大小失败：{error}"))?;
    if !output.status.success() {
        return Ok(0);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        .saturating_mul(1024))
}

fn deployed_record_v2(
    bundle: &Bundle,
    install: &Path,
    deploy_dir: &Path,
    project_id: &str,
    images: &BTreeMap<String, ImageSnapshot>,
    volumes: &[VolumePlan],
    transaction_id: &str,
) -> Result<DeployedRecord, String> {
    let mut record = bundle.deployed_record(project_id);
    record.schema_version = 2;
    record.images = images
        .values()
        .map(|image| DeployedImage {
            reference: image.reference.clone(),
            image_id: image.image_id.clone(),
            platform: image.platform.clone(),
        })
        .collect();
    let mut compose_files = Vec::new();
    for source in [
        bundle.descriptor.runtime.compose.trim(),
        bundle.effective_compose(),
    ] {
        if source.is_empty() {
            continue;
        }
        let Some(artifact) = bundle.artifact_for_source(source) else {
            continue;
        };
        let base = match artifact.root {
            ArtifactRoot::Install => install.to_path_buf(),
            ArtifactRoot::Deploy => deploy_dir.to_path_buf(),
        };
        let path = base.join(artifact.path.trim());
        if path.is_file() {
            compose_files.push(DeployedComposeFile {
                path: path.to_string_lossy().to_string(),
                sha256: sha256_path(&path)?,
            });
        }
    }
    record.compose_files = compose_files;
    let env = deploy_dir.join(".env");
    record.env_sha256 = if env.is_file() {
        sha256_path(&env)?
    } else {
        String::new()
    };
    record.data_volumes = volumes
        .iter()
        .map(|volume| volume.logical_name.clone())
        .collect();
    record.transaction_id = Some(transaction_id.to_string());
    Ok(record)
}

fn find_old_compose(
    install: &Path,
    deploy_dir: &Path,
    record: Option<&DeployedRecord>,
) -> Result<Option<PathBuf>, String> {
    if let Some(stored) = record
        .map(|record| record.effective_compose.as_str())
        .filter(|path| !path.is_empty())
    {
        let path = if path_guard::is_absolute(stored) {
            PathBuf::from(stored)
        } else {
            install.join(stored)
        };
        if !path_guard::path_is_within(deploy_dir, &path) || !path.is_file() {
            return Err(
                "上一版本的生效 Compose 清单缺失或位于其他目录，请确认旧部署配置后再部署"
                    .to_string(),
            );
        }
        return Ok(Some(path));
    }
    let active = docker_checked(
        &[
            "ps",
            "-a",
            "--filter",
            &format!(
                "label=com.docker.compose.project.working_dir={}",
                path_guard::docker_path(deploy_dir).display()
            ),
            "--format",
            "{{.Label \"com.docker.compose.project.config_files\"}}",
        ],
        "检查上一版本生效清单",
        Duration::from_secs(60),
    )?;
    for path in active
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        if !path_guard::path_is_within(deploy_dir, Path::new(path))
            && !path_guard::path_is_within(&install.join(".deploy-desktop"), Path::new(path))
        {
            return Err("上一版本引用部署目录外的 Compose 清单，本期无法安全恢复".to_string());
        }
    }
    let actual: BTreeSet<_> = active
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path_guard::path_is_within(deploy_dir, path))
        .collect();
    if actual.len() > 1 {
        return Err(
            "上一版本合并使用多份 Compose 清单，本期无法安全恢复，请先确认生效配置".to_string(),
        );
    }
    if let Some(path) = actual.into_iter().next() {
        if !path.is_file() {
            return Err("上一版本的实际 Compose 清单不存在，已停止部署".to_string());
        }
        return Ok(Some(path));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(record) = record {
        for compose in &record.compose_files {
            let path = PathBuf::from(&compose.path);
            if path.is_absolute() {
                candidates.push(path);
            } else {
                candidates.push(install.join(&path));
                candidates.push(deploy_dir.join(&path));
            }
        }
        for artifact in &record.artifacts {
            if !artifact.path.ends_with(".yml") && !artifact.path.ends_with(".yaml") {
                continue;
            }
            let base = match artifact.root.as_str() {
                "install" => install.to_path_buf(),
                "deploy" => deploy_dir.to_path_buf(),
                _ => continue,
            };
            let path = base.join(&artifact.path);
            candidates.push(path);
        }
    }
    for name in [
        "docker-compose.lan.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yaml",
        "compose.yml",
    ] {
        candidates.push(deploy_dir.join(name));
    }
    candidates.sort();
    candidates.dedup();
    let existing: Vec<_> = candidates
        .into_iter()
        .filter(|path| path.is_file())
        .collect();
    if existing.len() > 1 {
        return Err(
            "旧部署未记录实际生效的 Compose，且存在多份清单；请先确认旧版本的生效清单".to_string(),
        );
    }
    let selected = existing.into_iter().next();
    if selected
        .as_ref()
        .is_some_and(|path| !path_guard::path_is_within(deploy_dir, path))
    {
        return Err("上一版本 Compose 位于其他目录，本期不支持迁移部署目录".to_string());
    }
    if selected.is_none() && record.is_some() {
        return Err("旧部署记录存在，但找不到上一版本 Compose 清单，已停止部署".to_string());
    }
    Ok(selected)
}

fn ensure_no_symlink(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "无法读取交付包文件".to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("交付包包含符号链接，无法安全部署".to_string());
    }
    Ok(())
}

fn verify_service_declarations(bundle: &Bundle) -> Result<(), String> {
    let source = bundle.effective_compose();
    let path = bundle.root.join(source);
    let content = fs::read_to_string(&path)
        .map_err(|_| format!("无法读取交付包中的 Compose 清单 {source}"))?;
    let services: BTreeSet<String> = crate::core::compose::service_names(&content)
        .map_err(|_| "交付包 Compose 清单格式错误".to_string())?
        .into_iter()
        .collect();
    let missing: Vec<&str> = bundle
        .descriptor
        .runtime
        .services
        .iter()
        .map(|service| service.key.as_str())
        .filter(|key| !services.contains(*key))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "交付包声明的服务未出现在 Compose 清单：{}",
            missing.join("、")
        ))
    }
}

fn verify_all_declared_services(bundle: &Bundle, actual: &[String]) -> Result<(), String> {
    let undeclared: Vec<_> = actual
        .iter()
        .filter(|key| {
            !bundle
                .descriptor
                .runtime
                .services
                .iter()
                .any(|service| service.key == **key)
        })
        .collect();
    if !undeclared.is_empty() {
        return Err(format!(
            "Compose 包含未声明角色的服务：{}",
            undeclared
                .iter()
                .map(|key| key.as_str())
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    let missing: Vec<&str> = bundle
        .descriptor
        .runtime
        .services
        .iter()
        .map(|service| service.key.as_str())
        .filter(|key| !actual.iter().any(|item| item == key))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "交付包服务未出现在实际 Compose 配置：{}",
            missing.join("、")
        ))
    }
}

pub(crate) fn verify_declared_image_references(
    bundle: &Bundle,
    config: &Value,
) -> Result<(), String> {
    for service in active_service_configs(config) {
        let reference = service
            .get("image")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !bundle
            .descriptor
            .images
            .iter()
            .any(|image| image.reference == reference)
        {
            return Err(format!(
                "Compose 服务镜像 {reference} 未在交付包镜像清单中声明，请重新构建制品"
            ));
        }
    }
    Ok(())
}

fn compose_project_name(config: &Value) -> Option<String> {
    config
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn active_service_configs(config: &Value) -> Vec<&Value> {
    config
        .get("services")
        .and_then(Value::as_object)
        .map(|services| {
            services
                .values()
                .filter(|service| {
                    service
                        .get("profiles")
                        .and_then(Value::as_array)
                        .map(|profiles| profiles.is_empty())
                        .unwrap_or(true)
                })
                .collect()
        })
        .unwrap_or_default()
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

fn image_snapshots(
    install: &Path,
    config: &Value,
) -> Result<BTreeMap<String, ImageSnapshot>, String> {
    let bindings = super::deployment_image_bindings::local_bindings(install)?;
    let mut result = BTreeMap::new();
    for service in active_service_configs(config) {
        let reference = service
            .get("image")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Compose 服务未声明镜像".to_string())?;
        let identity = bindings
            .iter()
            .find(|binding| binding.reference == reference)
            .map(|binding| binding.image_id.as_str())
            .unwrap_or(reference);
        let output = docker_output(
            &[
                "image",
                "inspect",
                "--format",
                "{{.Id}}|{{.Os}}/{{.Architecture}}",
                "--",
                identity,
            ],
            &format!("检查镜像 {reference}"),
        )
        .map_err(|_| format!("目标 Docker 缺少镜像 {reference}，请先准备完整版本镜像"))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let (id, platform) = text
            .trim()
            .split_once('|')
            .ok_or_else(|| format!("镜像 {reference} 状态无法解析"))?;
        result.insert(
            reference.to_string(),
            ImageSnapshot {
                reference: reference.to_string(),
                image_id: id.to_string(),
                platform: platform.to_string(),
            },
        );
    }
    Ok(result)
}

fn previous_image_snapshots(
    install: &Path,
    config: &Value,
    record: Option<&DeployedRecord>,
) -> Result<Vec<ImageSnapshot>, String> {
    let recorded = record
        .map(|record| record.images.as_slice())
        .unwrap_or_default();
    let mut fallback: Option<BTreeMap<String, ImageSnapshot>> = None;
    let mut result = BTreeMap::new();
    for service in active_service_configs(config) {
        let reference = service
            .get("image")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "上一版本服务未声明镜像".to_string())?;
        if result.contains_key(reference) {
            continue;
        }
        if let Some(image) = recorded.iter().find(|image| image.reference == reference) {
            if !super::deployment_image_bindings::valid_image_id(&image.image_id) {
                return Err(format!("上一版本部署记录中的镜像 {reference} 标识无效"));
            }
            let inspected = inspect_image(&image.image_id)?
                .ok_or_else(|| format!("上一版本镜像 {reference} 已不存在，无法安全恢复"))?;
            result.insert(
                reference.to_string(),
                ImageSnapshot {
                    reference: reference.to_string(),
                    image_id: inspected.image_id,
                    platform: inspected.platform,
                },
            );
            continue;
        }
        if fallback.is_none() {
            fallback = Some(image_snapshots(install, config)?);
        }
        let image = fallback
            .as_ref()
            .and_then(|images| images.get(reference))
            .cloned()
            .ok_or_else(|| format!("无法确认上一版本镜像 {reference}"))?;
        result.insert(reference.to_string(), image);
    }
    Ok(result.into_values().collect())
}

fn inspect_image(reference: &str) -> Result<Option<ImageSnapshot>, String> {
    let output = docker_output(
        &[
            "image",
            "inspect",
            "--format",
            "{{.Id}}|{{.Os}}/{{.Architecture}}",
            "--",
            reference,
        ],
        &format!("检查镜像 {reference}"),
    );
    let output = match output {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let (id, platform) = text
        .trim()
        .split_once('|')
        .ok_or_else(|| format!("镜像 {reference} 状态无法解析"))?;
    Ok(Some(ImageSnapshot {
        reference: reference.to_string(),
        image_id: id.to_string(),
        platform: platform.to_string(),
    }))
}

fn compose_config_bundle(
    compose: &Path,
    deploy_dir: &Path,
    env_content: Option<&str>,
) -> Result<Value, String> {
    let mut command = crate::commands::docker_setup::docker_command();
    command
        .arg("compose")
        .arg("--project-directory")
        .arg(path_guard::docker_path(deploy_dir));
    let env_files = super::docker_ops::resolve_env_files_for(
        deploy_dir.parent().unwrap_or(deploy_dir),
        deploy_dir,
    );
    for env in env_files
        .iter()
        .filter(|env| env.file_name().and_then(|name| name.to_str()) != Some(".env.local"))
    {
        command.arg("--env-file").arg(path_guard::docker_path(&env));
    }
    let _env = if let Some(content) = env_content {
        let path = std::env::temp_dir().join(format!(
            "deploy-preflight-{}.env",
            crate::core::backup::unique_suffix()
        ));
        super::projects::write_private_env(&path, content)?;
        command
            .arg("--env-file")
            .arg(path_guard::docker_path(&path));
        Some(TemporaryEnvFile(path))
    } else {
        None
    };
    for env in env_files
        .iter()
        .filter(|env| env.file_name().and_then(|name| name.to_str()) == Some(".env.local"))
    {
        command.arg("--env-file").arg(path_guard::docker_path(&env));
    }
    command.arg("-f").arg(path_guard::docker_path(compose));
    run_compose_json(command)
}

fn compose_config(
    install: &Path,
    deploy_dir: &Path,
    compose: &Path,
    override_document: bool,
) -> Result<Value, String> {
    let mut command = compose_base_command(install, deploy_dir, compose);
    if override_document {
        if let Some(document) =
            super::deployment_image_bindings::local_override_document(install, compose, false)?
        {
            command.args(["-f", "-"]);
            return run_compose_json_with_input(command, document.into_bytes());
        }
    }
    run_compose_json(command)
}

fn run_compose_json(mut command: Command) -> Result<Value, String> {
    command.args(["config", "--format", "json"]);
    let output = process::run(
        command,
        process::Input::Bytes(vec![]),
        process::Options::default(),
    )?;
    if !output.status.success() {
        return Err(compose_error(
            "读取 Compose 配置",
            &String::from_utf8_lossy(&output.stderr),
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "无法解析 Compose 配置，请检查 Docker Compose 版本".to_string())
}

fn run_compose_json_with_input(mut command: Command, input: Vec<u8>) -> Result<Value, String> {
    command.args(["config", "--format", "json"]);
    let output = process::run(
        command,
        process::Input::Bytes(input),
        process::Options::default(),
    )?;
    if !output.status.success() {
        return Err(compose_error(
            "读取 Compose 配置",
            &String::from_utf8_lossy(&output.stderr),
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "无法解析 Compose 配置，请检查 Docker Compose 版本".to_string())
}

fn compose_base_command(install: &Path, deploy_dir: &Path, compose: &Path) -> Command {
    let mut command = crate::commands::docker_setup::docker_command();
    command
        .arg("compose")
        .arg("--project-directory")
        .arg(path_guard::docker_path(deploy_dir));
    for env in super::docker_ops::resolve_env_files_for(install, deploy_dir) {
        command.arg("--env-file").arg(path_guard::docker_path(&env));
    }
    command
        .arg("-f")
        .arg(path_guard::docker_path(compose))
        .current_dir(path_guard::docker_path(deploy_dir));
    command
}

fn compose_checked(
    install: &Path,
    deploy_dir: &Path,
    compose: &Path,
    action: &[&str],
    override_document: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    let mut command = compose_base_command(install, deploy_dir, compose);
    let input = if let Some(document) = override_document {
        command.args(["-f", "-"]);
        document.as_bytes().to_vec()
    } else {
        Vec::new()
    };
    command.args(action);
    let output = process::run(
        command,
        process::Input::Bytes(input),
        process::Options {
            timeout,
            ..process::Options::default()
        },
    )?;
    if output.status.success() {
        Ok(format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    } else {
        Err(compose_error(
            "Compose 操作",
            &String::from_utf8_lossy(&output.stderr),
        ))
    }
}

fn compose_error(action: &str, stderr: &str) -> String {
    let detail = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("；");
    if detail.is_empty() {
        format!("{action}失败，请检查 Docker 与 Compose 状态")
    } else {
        format!("{action}失败：{detail}")
    }
}

fn wait_for_services(
    install: &Path,
    deploy_dir: &Path,
    compose: &Path,
    services: &[String],
    frontend: Option<(&str, u16)>,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let mut unready = Vec::new();
        for service in services {
            let ids = compose_checked(
                install,
                deploy_dir,
                compose,
                &["ps", "-a", "-q", service],
                None,
                Duration::from_secs(60),
            )
            .unwrap_or_default();
            if ids.trim().is_empty() {
                unready.push(service.clone());
                continue;
            }
            for id in ids.lines().map(str::trim).filter(|id| !id.is_empty()) {
                let output = docker_output(
                    &[
                        "inspect",
                        "--format",
                        "{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}",
                        id,
                    ],
                    "检查容器状态",
                )?;
                let state = String::from_utf8_lossy(&output.stdout);
                let (status, health) = state.trim().split_once('|').unwrap_or(("missing", "none"));
                if status != "running" || matches!(health, "unhealthy" | "starting") {
                    unready.push(service.clone());
                }
            }
        }
        let http_unready =
            unready.is_empty() && frontend.map(|(_, port)| !http_ready(port)).unwrap_or(false);
        if unready.is_empty() && !http_unready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let mut detail = String::new();
            for service in unready.iter().take(3) {
                let logs = compose_checked(
                    install,
                    deploy_dir,
                    compose,
                    &["logs", "--tail", "30", service],
                    None,
                    Duration::from_secs(60),
                )
                .unwrap_or_default();
                detail.push_str(&format!("\n{service}: {logs}"));
            }
            if http_unready {
                let (service, port) = frontend.ok_or_else(|| "网页服务端口未声明".to_string())?;
                return Err(format!(
                    "网页服务 {service} 未通过 HTTP 就绪检查：127.0.0.1:{port}{detail}"
                ));
            }
            return Err(format!(
                "服务未在等待时间内就绪：{}{}",
                unready.join("、"),
                detail
            ));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn frontend_probe(service: &str, port: Option<u16>) -> Option<(&str, u16)> {
    if service.trim().is_empty() {
        return None;
    }
    port.map(|port| (service, port))
}

fn http_ready(port: u16) -> bool {
    let Some(mut stream) = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(3),
    )
    .ok() else {
        return false;
    };
    use std::io::{Read, Write};
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let request = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if stream.write_all(request).is_err() {
        return false;
    }
    let mut response = [0u8; 64];
    let Ok(read) = stream.read(&mut response) else {
        return false;
    };
    let status = String::from_utf8_lossy(&response[..read]);
    status
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .map(|code| (200..400).contains(&code))
        .unwrap_or(false)
}

fn docker_output(args: &[&str], purpose: &str) -> Result<std::process::Output, String> {
    let mut command = crate::commands::docker_setup::docker_command();
    command.args(args);
    let output = process::run(
        command,
        process::Input::Bytes(vec![]),
        process::Options {
            timeout: Duration::from_secs(600),
            ..process::Options::default()
        },
    )?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{purpose}失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn docker_checked(args: &[&str], purpose: &str, timeout: Duration) -> Result<String, String> {
    let mut command = crate::commands::docker_setup::docker_command();
    command.args(args);
    let output = process::run(
        command,
        process::Input::Bytes(vec![]),
        process::Options {
            timeout,
            ..process::Options::default()
        },
    )?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(format!(
            "{purpose}失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn check_local_docker() -> Result<(), String> {
    let context = docker_checked(
        &[
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ],
        "读取 Docker 连接",
        Duration::from_secs(30),
    )?;
    let endpoint = std::env::var("DOCKER_CONTEXT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|_| context.clone())
        .or_else(|| std::env::var("DOCKER_HOST").ok())
        .unwrap_or(context);
    if !endpoint.trim().starts_with("unix://") && !endpoint.trim().starts_with("npipe://") {
        return Err("完整版本部署仅支持本机 Docker，请切换本机 Docker 连接".to_string());
    }
    docker_checked(
        &["info", "--format", "{{.ServerVersion}}"],
        "检查 Docker",
        Duration::from_secs(30),
    )?;
    docker_checked(
        &["compose", "version"],
        "检查 Docker Compose",
        Duration::from_secs(30),
    )?;
    Ok(())
}

struct TemporaryEnvFile(PathBuf);

impl Drop for TemporaryEnvFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn resolve_install_root(raw: &str, must_exist: bool) -> Result<PathBuf, String> {
    let path = Path::new(raw.trim());
    if !path.is_absolute() || path_guard::has_parent_dir(path) {
        return Err("部署目录必须使用不含上级跳转的绝对路径".to_string());
    }
    if path.exists() {
        let canonical =
            fs::canonicalize(path).map_err(|_| "部署目录不存在或无法访问".to_string())?;
        if !canonical.is_dir() || canonical.parent().is_none() {
            return Err("部署目录无效，请选择具体目录".to_string());
        }
        for part in canonical.ancestors() {
            if fs::symlink_metadata(part)
                .map(|meta| meta.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err("部署路径含符号链接，请选择实际目录".to_string());
            }
        }
        return Ok(canonical);
    }
    if must_exist {
        return Err("部署目录不存在".to_string());
    }
    let parent = path.parent().ok_or_else(|| "部署目录无效".to_string())?;
    let canonical_parent =
        fs::canonicalize(parent).map_err(|_| "部署目录的上级目录不存在".to_string())?;
    if canonical_parent.parent().is_none() {
        return Err("部署目录不能是系统根目录".to_string());
    }
    Ok(canonical_parent.join(path.file_name().ok_or_else(|| "部署目录无效".to_string())?))
}

fn replace_path(source: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建部署目录 {} 失败：{error}", parent.display()))?;
    }
    remove_target(target)?;
    if source.is_dir() {
        copy_dir(source, target)
    } else {
        deployment_tx::retry_file_operation(|| fs::copy(source, target))
            .map(|_| ())
            .map_err(|error| format!("写入部署文件 {} 失败：{error}", target.display()))
    }
}

fn copy_dir(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target)
        .map_err(|error| format!("创建部署目录 {} 失败：{error}", target.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("读取交付目录 {} 失败：{error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("读取交付文件失败：{error}"))?;
        let destination = target.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| format!("读取交付文件类型失败：{error}"))?
            .is_dir()
        {
            copy_dir(&entry.path(), &destination)?;
        } else {
            deployment_tx::retry_file_operation(|| fs::copy(entry.path(), &destination))
                .map_err(|error| format!("写入部署文件失败：{error}"))?;
        }
    }
    Ok(())
}

fn remove_target(path: &Path) -> Result<(), String> {
    deployment_tx::remove_deployment_path(path)
}

fn tree_bytes(path: &Path) -> Result<u64, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("无法读取待快照路径：{error}")),
    };
    if metadata.file_type().is_symlink() {
        return Err("目标受管目录包含符号链接，无法安全部署".to_string());
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path).map_err(|error| format!("无法读取待快照目录：{error}"))?
    {
        let entry = entry.map_err(|error| format!("无法读取待快照文件：{error}"))?;
        total = total.saturating_add(tree_bytes(&entry.path())?);
    }
    Ok(total)
}

fn minimal_retired_paths(paths: BTreeSet<String>) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for path in paths {
        let covered = result
            .iter()
            .any(|parent| path == *parent || path.starts_with(&format!("{parent}/")));
        if !covered {
            result.push(path);
        }
    }
    result
}

fn ensure_preserved_paths_untouched(
    paths: &BTreeSet<String>,
    bundle: &Bundle,
) -> Result<(), String> {
    let preserved = bundle
        .descriptor
        .runtime
        .data
        .as_ref()
        .map(|data| data.paths.iter().map(|path| path.path.trim()))
        .into_iter()
        .flatten()
        .filter(|path| !path.is_empty());
    for preserve in preserved {
        for path in paths {
            if path == preserve
                || path.starts_with(&format!("{preserve}/"))
                || preserve.starts_with(&format!("{path}/"))
            {
                return Err(format!(
                    "部署受管路径 {path} 与保留数据目录 {preserve} 重叠，已阻止部署以保护现有数据"
                ));
            }
        }
    }
    Ok(())
}

fn available_disk_bytes(path: &Path) -> Result<u64, String> {
    let anchor = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    #[cfg(unix)]
    {
        let output = Command::new("df")
            .arg("-Pk")
            .arg(&anchor)
            .output()
            .map_err(|_| "无法检查部署分区空间".to_string())?;
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines()
            .last()
            .and_then(|line| line.split_whitespace().nth(3))
            .and_then(|value| value.parse::<u64>().ok())
            .map(|kb| kb * 1024)
            .ok_or_else(|| "无法检查部署分区空间".to_string())
    }
    #[cfg(windows)]
    {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-PSDrive -PSProvider FileSystem | Select-Object Root,Free | ConvertTo-Json -Compress",
            ])
            .output()
            .map_err(|_| "无法检查部署分区空间".to_string())?;
        let drives: Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| "无法检查部署分区空间".to_string())?;
        let entries: Vec<&Value> = match drives.as_array() {
            Some(entries) => entries.iter().collect(),
            None => vec![&drives],
        };
        let root = anchor
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase();
        entries
            .iter()
            .find_map(|drive| {
                let drive_root = drive.get("Root")?.as_str()?.to_lowercase();
                root.starts_with(&drive_root)
                    .then(|| drive.get("Free").and_then(Value::as_u64))
                    .flatten()
            })
            .ok_or_else(|| "无法检查部署分区空间".to_string())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = anchor;
        Ok(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_compose_uses_recorded_variant_instead_of_filename_order() {
        let install = std::env::temp_dir().join(format!(
            "previous-compose-{}",
            crate::core::backup::unique_suffix()
        ));
        let deploy = install.join("deploy");
        fs::create_dir_all(&deploy).unwrap();
        fs::write(deploy.join("app.yml"), "services: {}").unwrap();
        fs::write(deploy.join("z-variant.yml"), "services: {}").unwrap();
        let mut record: DeployedRecord = serde_json::from_value(serde_json::json!({
            "schemaVersion": 2, "projectId": "demo", "product": "demo", "version": "1",
            "installedAt": "", "artifacts": [],
            "effectiveCompose": "deploy/z-variant.yml"
        }))
        .unwrap();
        assert_eq!(
            find_old_compose(&install, &deploy, Some(&record)).unwrap(),
            Some(deploy.join("z-variant.yml"))
        );
        record.effective_compose = "stack/old-compose.yml".to_string();
        assert!(find_old_compose(&install, &deploy, Some(&record)).is_err());
        fs::remove_dir_all(install).unwrap();
    }

    #[test]
    fn frontend_ports_prefer_declared_http_port_and_reject_ambiguity() {
        let config = serde_json::json!({"services": {"frontend": {"ports": [
            {"published": "9090", "target": 9090, "protocol": "tcp"},
            {"published": "8080", "target": 80, "protocol": "tcp"}
        ]}}});
        assert_eq!(
            published_frontend_port(&config, "frontend", Some(8080)),
            Some(8080)
        );
        assert_eq!(
            published_frontend_port(&config, "frontend", Some(18080)),
            None
        );
    }

    #[test]
    fn persistent_mount_contract_matches_shared_cases() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/deployment/mounts.json")).unwrap();
        let contract = &fixture["contract"];
        let strings = |key: &str| {
            contract[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        for case in fixture["cases"].as_array().unwrap() {
            let result = validate_mount_contract(
                &case["config"],
                &strings("volumes"),
                &strings("dataServices"),
                &strings("preserve"),
                &strings("managed"),
            );
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}: {:?}",
                case["name"],
                result
            );
        }
    }

    #[test]
    fn image_overrides_use_frozen_previous_image_ids() {
        let reference = format!("demo/api@sha256:{}", "a".repeat(64));
        let old_id = format!("sha256:{}", "b".repeat(64));
        let config = serde_json::json!({"services": {"api": {"image": reference}}});
        let images = [ImageSnapshot {
            reference,
            image_id: old_id.clone(),
            platform: "linux/amd64".to_string(),
        }];
        let document = image_override_document(&config, &images).unwrap().unwrap();
        let parsed: Value = serde_json::from_str(&document).unwrap();
        assert_eq!(parsed["services"]["api"]["image"], old_id);
        assert_eq!(parsed["services"]["api"]["pull_policy"], "never");
        assert!(image_override_document(&config, &[]).is_err());
    }

    #[test]
    fn install_root_rejects_traversal_and_roots() {
        for path in ["", "/", "relative", "/tmp/../escape"] {
            assert!(resolve_install_root(path, false).is_err(), "{path}");
        }
    }

    #[test]
    fn active_services_skip_profiles() {
        let config = serde_json::json!({
            "services": {
                "api": {"image": "demo/api:1"},
                "tools": {"image": "demo/tools:1", "profiles": ["debug"]}
            }
        });
        assert_eq!(active_services(&config), vec!["api".to_string()]);
    }

    #[test]
    fn first_deploy_can_resolve_a_missing_install_directory() {
        let parent = std::env::temp_dir().join(format!(
            "version-deploy-parent-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&parent).unwrap();
        let target = parent.join("app");
        let resolved = resolve_install_root(&target.to_string_lossy(), false).unwrap();
        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some("app")
        );
        assert!(resolve_install_root(&target.to_string_lossy(), true).is_err());
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn data_volume_resolution_rejects_external_volumes() {
        let config = serde_json::json!({
            "volumes": {
                "db-data": {"name": "demo_db-data"},
                "shared": {"name": "shared", "external": true}
            }
        });
        assert_eq!(
            resolved_volume_name(&config, "db-data").unwrap(),
            "demo_db-data"
        );
        assert!(resolved_volume_name(&config, "shared")
            .unwrap_err()
            .contains("external"));
    }

    #[test]
    fn frontend_probe_requires_a_declared_port() {
        assert_eq!(
            frontend_probe("frontend", Some(18080)),
            Some(("frontend", 18080))
        );
        assert_eq!(frontend_probe("frontend", None), None);
        assert_eq!(frontend_probe("", Some(18080)), None);
    }

    #[test]
    fn journal_compose_paths_resolve_from_the_install_root() {
        let path = transaction_compose_path(Path::new("/srv/app"), "deploy/docker-compose.yml");
        assert_eq!(path, Path::new("/srv/app/deploy/docker-compose.yml"));
    }

    #[test]
    fn retired_paths_collapse_nested_entries() {
        let paths = [
            "config".to_string(),
            "config/old.yml".to_string(),
            "site/old".to_string(),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            minimal_retired_paths(paths),
            vec!["config".to_string(), "site/old".to_string()]
        );
    }

    #[test]
    fn compose_project_containers_must_belong_to_the_current_deploy_directory() {
        let install = Path::new("/srv/app");
        let deploy = Path::new("/srv/app/deploy");
        assert_eq!(
            validate_project_container_scope(
                "demo",
                install,
                deploy,
                "/srv/app/deploy|/srv/app/deploy/docker-compose.yml\n"
            )
            .unwrap(),
            true
        );
        assert!(validate_project_container_scope(
            "demo",
            install,
            deploy,
            "/srv/other/deploy|/srv/other/deploy/docker-compose.yml\n"
        )
        .unwrap_err()
        .contains("其他部署目录"));
        assert!(validate_project_container_scope("demo", install, deploy, "|\n")
            .unwrap_err()
            .contains("工作目录标签"));
    }
}
