//! Remote full-version deployment adapter.
//!
//! Artifacts are staged under the deployment transaction before the target is stopped.
//! The destructive worker runs detached and publishes a small status file so the
//! desktop and MCP adapters can reconnect without terminating an in-flight deployment.

use super::bundle::{normalize_install_dir, validate_install_dir};
use super::transfer::{upload_dir_replace, upload_file_with_progress};
use crate::commands::bundle::{
    default_components, sha256_hex, ArtifactRoot, Bundle, DeployedComposeFile, DeployedImage,
    DeployedRecord, DEPLOYED_RECORD_DIR, DEPLOYED_RECORD_FILE,
};
use crate::commands::deployment_tx::{DeploymentTransaction, ImageSnapshot};
use crate::commands::remote_ssh::{
    resolve_existing_key, run_ssh_script, run_ssh_script_with_timeout, shell_quote,
};
use crate::commands::version_deploy::VersionDeployResult;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::Emitter;

pub(crate) fn inspect_remote_transaction(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install_dir: &str,
) -> Result<Option<DeploymentTransaction>, String> {
    let install = normalize_install_dir(install_dir);
    if install.is_empty() {
        return Ok(None);
    }
    let script = format!(
        r#"set -e
ROOT={root}
[ -d "$ROOT" ] || exit 0
find "$ROOT" -mindepth 1 -maxdepth 1 -type d -name 'deploy-*' | sort -r | while IFS= read -r dir; do
  [ -f "$dir/journal.json" ] || {{ echo '部署事务缺少 journal，请检查现场后再部署' >&2; exit 4; }}
  printf 'JOURNAL_B64='
  base64 < "$dir/journal.json" | tr -d '\n'
  printf '\n'
  if [ -f "$dir/status.env" ]; then
    printf 'STATUS_B64='
    base64 < "$dir/status.env" | tr -d '\n'
    printf '\n'
  fi
done
"#,
        root = shell_quote(&format!("{install}/.deploy-desktop/transactions"))
    );
    let output = run_ssh_script(host, port, user, key, &script)?;
    if !output.status.success() {
        return Err("读取目标机部署事务失败，请检查 SSH 连接与目录权限".to_string());
    }
    parse_remote_journal(&String::from_utf8_lossy(&output.stdout))
}

fn parse_remote_journal(raw: &str) -> Result<Option<DeploymentTransaction>, String> {
    let mut transactions: Vec<DeploymentTransaction> = Vec::new();
    for line in raw.lines() {
        if let Some(journal) = line.strip_prefix("JOURNAL_B64=") {
            transactions.push(
                serde_json::from_slice(&decode_base64(journal.trim())?)
                    .map_err(|_| "目标机部署事务记录格式无效".to_string())?,
            );
        } else if let Some(status) = line.strip_prefix("STATUS_B64=") {
            if let Some(transaction) = transactions.last_mut() {
                apply_status(transaction, &decode_base64(status.trim())?)?;
            }
        }
    }
    Ok(transactions
        .iter()
        .find(|transaction| !matches!(transaction.state.as_str(), "committed" | "recovered"))
        .cloned()
        .or_else(|| transactions.into_iter().next()))
}

fn apply_status(transaction: &mut DeploymentTransaction, raw: &[u8]) -> Result<(), String> {
    let text = String::from_utf8(raw.to_vec())
        .map_err(|_| "目标机部署事务状态不是有效 UTF-8".to_string())?;
    let values: BTreeMap<String, String> = text
        .lines()
        .filter_map(|line| {
            line.split_once('=')
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    if let Some(phase) = values.get("phase") {
        transaction.phase = phase.clone();
    }
    if let Some(state) = values.get("state") {
        transaction.state = state.clone();
    }
    if let Some(updated) = values.get("updatedAt") {
        transaction.updated_at = updated.clone();
    }
    if let Some(started) = values.get("deploymentStarted") {
        transaction.deployment_started = started == "yes";
    }
    transaction.error = values
        .get("errorBase64")
        .filter(|value| !value.is_empty())
        .map(|value| decode_base64(value))
        .transpose()?
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string());
    transaction.recovery_error = values
        .get("recoveryErrorBase64")
        .filter(|value| !value.is_empty())
        .map(|value| decode_base64(value))
        .transpose()?
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string());
    Ok(())
}

fn decode_base64(raw: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in raw.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => continue,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Ok(output)
}

pub(crate) fn recover_remote_transaction(
    window: &tauri::Window,
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<&str>,
    install_dir: &str,
) -> Result<VersionDeployResult, String> {
    let started = Instant::now();
    let key = resolve_existing_key(key_path.map(str::to_string), host)?;
    let install = normalize_install_dir(install_dir);
    validate_install_dir(&install)?;
    let mut transaction = inspect_remote_transaction(host, port, user, key.as_deref(), &install)?
        .ok_or_else(|| "目标机没有可恢复的部署事务".to_string())?;
    if transaction.state == "recovery_failed" {
        return Err("该事务上次恢复失败，请按快照目录人工处理，系统不会自动重试".to_string());
    }
    if transaction.state == "recovering" {
        return Err("该事务正在恢复或上次恢复被中断，请先检查事务现场后再处理".to_string());
    }
    if !matches!(transaction.state.as_str(), "running" | "failed" | "pending") {
        return Err(format!(
            "事务 {} 当前状态为 {}，无需恢复",
            transaction.id, transaction.state
        ));
    }
    if remote_deploy_lock_active(host, port, user, key.as_deref(), &install)? {
        return Err("目标机部署任务仍在执行，请等待事务结束或确认进程已退出后再恢复".to_string());
    }
    let txn = format!("{install}/.deploy-desktop/transactions/{}", transaction.id);
    let worker = format!("{txn}/worker.sh");
    let exists = run_ssh_script(
        host,
        port,
        user,
        key.as_deref(),
        &format!(
            "test -f {} && test -f {}/incoming/volume-specs.tsv && echo READY",
            shell_quote(&worker),
            shell_quote(&txn)
        ),
    )?;
    if !exists.status.success() || !String::from_utf8_lossy(&exists.stdout).contains("READY") {
        return Err(format!(
            "部署事务缺少恢复脚本或快照清单，现场保留在 {}",
            transaction.snapshot_dir
        ));
    }
    progress(window, "恢复上一版本", 8, "running", "启动远端恢复任务");
    launch_worker(
        host,
        port,
        user,
        key.as_deref(),
        &txn,
        "recover",
        &[
            &install,
            &transaction.deploy_dir,
            &txn,
            if transaction.had_deployment {
                "yes"
            } else {
                "no"
            },
            &transaction.previous_compose,
            &transaction.new_compose,
            &transaction.frontend_service,
            &transaction
                .frontend_port
                .map(|port| port.to_string())
                .unwrap_or_default(),
            &transaction.services.join(" "),
            &transaction.previous_services.join(" "),
            &format!("{txn}/image-override.compose.json"),
            &format!("{txn}/old-image-override.compose.json"),
        ],
    )?;
    let final_transaction = poll_remote_transaction(
        window,
        host,
        port,
        user,
        key.as_deref(),
        &install,
        Duration::from_secs(6 * 60 * 60),
    )?;
    transaction = final_transaction;
    let log = read_remote_tail(host, port, user, key.as_deref(), &txn)?;
    let recovered = transaction.state == "recovered"
        && transaction.had_deployment
        && transaction.deployment_started;
    let first_deploy_cleaned = transaction.state == "recovered" && !transaction.had_deployment;
    Ok(VersionDeployResult {
        success: false,
        recovered,
        recovery_failed: transaction.state != "recovered",
        stage: "recovering".to_string(),
        version: transaction.version.clone(),
        previous_version: transaction.previous_version.clone(),
        transaction_id: transaction.id.clone(),
        snapshot_dir: transaction.snapshot_dir.clone(),
        error: if recovered || first_deploy_cleaned {
            transaction.error.clone()
        } else {
            Some(
                transaction
                    .recovery_error
                    .clone()
                    .unwrap_or_else(|| "远端恢复未完成".to_string()),
            )
        },
        output: if !transaction.deployment_started {
            format!(
                "本次事务在修改目标前已停止，无需恢复服务、文件或数据卷\n事务：{}\n{}",
                transaction.id, log
            )
        } else if recovered {
            format!(
                "已恢复上一版本 {}\n事务：{}\n{}",
                if transaction.previous_version.is_empty() {
                    "未知版本"
                } else {
                    &transaction.previous_version
                },
                transaction.id,
                log
            )
        } else if first_deploy_cleaned {
            format!("首次部署事务已清理，未保留上一版本\n{}", log)
        } else {
            format!(
                "恢复未完成，现场保留在 {}\n{}",
                transaction.snapshot_dir, log
            )
        },
        duration_secs: started.elapsed().as_secs(),
        status: Some(transaction),
    })
}

pub(crate) fn deploy_remote_full(
    window: &tauri::Window,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
    install_dir: String,
    dry_run: bool,
) -> Result<VersionDeployResult, String> {
    let started = Instant::now();
    progress(window, "预检完整版本", 3, "running", "核对交付包与目标机");
    let key = resolve_existing_key(key_path, &host)?;
    let install = normalize_install_dir(&install_dir);
    validate_install_dir(&install)?;
    let bundle = Bundle::load(Path::new(bundle_dir.trim()))?;
    bundle.ensure_compatible()?;
    bundle.verify_payload()?;
    bundle.ensure_full_deploy_metadata()?;
    if let Some(missing) = verify_remote_service_declarations(&bundle)? {
        return Err(missing);
    }
    let transaction = inspect_remote_transaction(&host, port, &user, key.as_deref(), &install)?;
    if let Some(transaction) = transaction
        .filter(|transaction| !matches!(transaction.state.as_str(), "committed" | "recovered"))
    {
        if transaction.state == "recovery_failed" {
            return Err(format!(
                "检测到恢复失败的部署事务 {}（阶段：{}）。现场保留在 {}，请人工处理后再部署",
                transaction.id, transaction.phase, transaction.snapshot_dir
            ));
        }
        if transaction.state == "recovering" {
            return Err(format!(
                "检测到恢复被中断的部署事务 {}（阶段：{}）。现场保留在 {}，请检查恢复日志并人工处理后再部署",
                transaction.id, transaction.phase, transaction.snapshot_dir
            ));
        }
        return Err(format!(
            "目标机存在未完成部署事务 {}（阶段：{}），请先恢复上一版本",
            transaction.id, transaction.phase
        ));
    }
    let plan = remote_preflight(&host, port, &user, key.as_deref(), &bundle, &install)?;
    if dry_run {
        progress(
            window,
            "预检通过",
            100,
            "success",
            "未上传部署文件，也未停止目标服务",
        );
        return Ok(VersionDeployResult {
            success: true,
            recovered: false,
            recovery_failed: false,
            stage: "preparing".to_string(),
            version: bundle.version().to_string(),
            previous_version: plan.previous_version.clone(),
            transaction_id: String::new(),
            snapshot_dir: String::new(),
            error: None,
            output: format!(
                "预检通过：目标 {}；版本 {}；{} 个服务；数据卷：{}；预计备份空间约 {:.1} GB。未上传部署文件，也未停止服务。",
                install,
                bundle.version(),
                plan.services.len(),
                plan.volumes
                    .iter()
                    .map(|volume| volume.logical.clone())
                    .collect::<Vec<_>>()
                    .join("、"),
                plan.required_backup_bytes as f64 / 1024.0 / 1024.0 / 1024.0
            ),
            duration_secs: started.elapsed().as_secs(),
            status: None,
        });
    }

    let id = format!(
        "deploy-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or(0),
        crate::core::backup::unique_suffix()
    );
    let txn = format!("{install}/.deploy-desktop/transactions/{id}");
    let incoming = format!("{txn}/incoming");
    let mut upload_cleanup = Some(RemoteCleanup {
        host: host.clone(),
        port,
        user: user.clone(),
        key: key.clone(),
        path: txn.clone(),
    });
    ensure_remote_dir(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{incoming}/tree"),
    )?;
    progress(
        window,
        "上传完整版本",
        18,
        "running",
        "上传全部交付文件到事务暂存区",
    );
    let selected = bundle.select_components(&default_components(&bundle))?;
    let deploy_dir = remote_join(&install, &bundle.deploy_source_rel());
    let artifacts =
        bundle.resolve_artifacts(&selected, Path::new(&install), Path::new(&deploy_dir))?;
    let mut upload_paths = Vec::new();
    for (index, artifact) in artifacts.iter().enumerate() {
        let relative = artifact
            .target
            .strip_prefix(Path::new(&install))
            .map_err(|_| "远程产物路径超出部署目录".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        upload_paths.push(relative.clone());
        let remote = format!("{incoming}/tree/{relative}");
        let percent = 18 + index * 28 / artifacts.len().max(1);
        progress(
            window,
            "上传完整版本",
            percent,
            "running",
            &format!("上传 {}", artifact.label),
        );
        match artifact.kind {
            crate::commands::bundle::ArtifactKind::File => upload_file_with_progress(
                &host,
                port,
                &user,
                key.as_deref(),
                &artifact.source,
                &remote,
                None,
            )?,
            crate::commands::bundle::ArtifactKind::Dir => upload_dir_replace(
                &host,
                port,
                &user,
                key.as_deref(),
                &artifact.source,
                &remote,
            )?,
        }
    }
    let env_content = plan.env_content.clone();
    if !env_content.trim().is_empty() {
        write_remote_text(
            &host,
            port,
            &user,
            key.as_deref(),
            &format!("{incoming}/env"),
            &env_content,
            true,
        )?;
        write_remote_text(
            &host,
            port,
            &user,
            key.as_deref(),
            &format!("{incoming}/env-present"),
            "yes\n",
            false,
        )?;
    }
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{incoming}/upload-paths.txt"),
        &format!("{}\n", upload_paths.join("\n")),
        false,
    )?;
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{incoming}/managed-paths.txt"),
        &format!("{}\n", plan.managed_paths.join("\n")),
        false,
    )?;
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{incoming}/remove-paths.txt"),
        &format!("{}\n", plan.retired_paths.join("\n")),
        false,
    )?;
    let volume_specs = plan
        .volumes
        .iter()
        .map(|volume| {
            format!(
                "{}\t{}\t{}\t{}",
                volume.logical,
                volume.actual,
                volume.image,
                if volume.exists { "yes" } else { "no" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{incoming}/volume-specs.tsv"),
        &format!("{volume_specs}\n"),
        false,
    )?;
    let mut record = remote_deployed_record(
        &bundle,
        &install,
        &deploy_dir,
        &plan.new_images,
        &plan.volumes,
        &id,
        &env_content,
    )?;
    record.transaction_id = Some(id.clone());
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{incoming}/deployed.json"),
        &serde_json::to_string_pretty(&record).map_err(|_| "生成部署记录失败".to_string())?,
        true,
    )?;
    let transaction = DeploymentTransaction {
        schema_version: 1,
        id: id.clone(),
        product: bundle.product().to_string(),
        version: bundle.version().to_string(),
        previous_version: plan.previous_version.clone(),
        install_dir: install.clone(),
        deploy_dir: deploy_dir.clone(),
        phase: "preparing".to_string(),
        state: "pending".to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        snapshot_dir: format!("{txn}/snapshot"),
        had_deployment: plan.had_deployment,
        project_name: plan.project_name.clone(),
        previous_compose: plan.old_compose.clone(),
        new_compose: plan.new_compose.clone(),
        files: Vec::new(),
        volumes: Vec::new(),
        old_images: plan.old_images.clone(),
        services: plan.services.clone(),
        previous_services: plan.old_services.clone(),
        frontend_service: plan.frontend_service.clone(),
        frontend_port: bundle
            .descriptor
            .runtime
            .services
            .iter()
            .find(|service| service.key.eq_ignore_ascii_case("frontend"))
            .and_then(|service| service.port),
        startup_attempted: false,
        deployment_started: false,
        applied_paths: Vec::new(),
        error: None,
        recovery_error: None,
    };
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{txn}/journal.json"),
        &serde_json::to_string_pretty(&transaction).map_err(|_| "生成部署事务失败".to_string())?,
        false,
    )?;
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{txn}/worker.sh"),
        &format!(
            "{}\n{}",
            crate::commands::docker_setup::REMOTE_DOCKER_PREAMBLE,
            include_str!("../../../../shell/version-deploy.sh")
        ),
        false,
    )?;
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{txn}/old-frontend-port"),
        &plan
            .old_frontend_port
            .map(|port| port.to_string())
            .unwrap_or_default(),
        false,
    )?;
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{txn}/baseline.sh"),
        include_str!("../../../../shell/deployment-baseline.sh"),
        false,
    )?;
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &format!("{incoming}/expected-baseline"),
        &plan.baseline,
        false,
    )?;
    let override_file = format!("{txn}/image-override.compose.json");
    write_remote_text(
        &host,
        port,
        &user,
        key.as_deref(),
        &override_file,
        &plan.image_override,
        false,
    )?;
    let old_override_file = format!("{txn}/old-image-override.compose.json");
    if !plan.old_image_override.is_empty() {
        write_remote_text(
            &host,
            port,
            &user,
            key.as_deref(),
            &old_override_file,
            &plan.old_image_override,
            false,
        )?;
    }
    if let Some(guard) = upload_cleanup.take() {
        std::mem::forget(guard);
    }
    progress(
        window,
        "开始部署事务",
        50,
        "running",
        "目标机将在后台停止服务并建立快照",
    );
    launch_worker(
        &host,
        port,
        &user,
        key.as_deref(),
        &txn,
        "deploy",
        &[
            &install,
            &deploy_dir,
            &txn,
            if plan.had_deployment { "yes" } else { "no" },
            &plan.old_compose,
            &plan.new_compose,
            &plan.frontend_service,
            &plan
                .frontend_port
                .map(|port| port.to_string())
                .unwrap_or_default(),
            &plan.services.join(" "),
            &plan.old_services.join(" "),
            &override_file,
            &old_override_file,
            &plan
                .old_frontend_port
                .map(|port| port.to_string())
                .unwrap_or_default(),
        ],
    )?;
    let transaction = poll_remote_transaction(
        window,
        &host,
        port,
        &user,
        key.as_deref(),
        &install,
        Duration::from_secs(12 * 60 * 60),
    )?;
    let log = read_remote_tail(&host, port, &user, key.as_deref(), &txn)?;
    let success = transaction.state == "committed";
    if success {
        let _ = run_ssh_script(
            &host,
            port,
            &user,
            key.as_deref(),
            &format!("rm -rf -- {}", shell_quote(&format!("{txn}/incoming"))),
        );
    }
    let recovered =
        transaction.state == "recovered" && plan.had_deployment && transaction.deployment_started;
    let first_deploy_cleaned = transaction.state == "recovered" && !plan.had_deployment;
    let recovery_failed = transaction.state == "recovery_failed";
    let stage = transaction.phase.clone();
    let output = if success {
        format!(
            "完整版本部署成功：{}；事务 {}\n{}",
            bundle.version(),
            transaction.id,
            log
        )
    } else if !transaction.deployment_started {
        format!(
            "本次部署在修改目标前已停止，目标文件与数据未被本次部署修改。失败原因：{}\n{}",
            transaction.error.as_deref().unwrap_or("请查看执行详情"),
            log
        )
    } else if recovered {
        format!(
            "新版本部署失败，已恢复上一版本 {}\n失败原因：{}\n{}",
            if plan.previous_version.is_empty() {
                "未知版本"
            } else {
                &plan.previous_version
            },
            transaction.error.as_deref().unwrap_or("请查看事务日志"),
            log
        )
    } else if first_deploy_cleaned {
        format!(
            "首次部署失败，已清理本次创建的服务与数据卷\n失败原因：{}\n{}",
            transaction.error.as_deref().unwrap_or("请查看事务日志"),
            log
        )
    } else if recovery_failed {
        format!(
            "新版本部署失败，自动恢复未完成。现场保留在 {}\n{}",
            transaction.snapshot_dir, log
        )
    } else {
        format!("部署事务结束但状态未确认：{}\n{}", transaction.state, log)
    };
    if success {
        progress(
            window,
            "部署完成",
            100,
            "success",
            "完整版本已启动并通过就绪检查",
        );
    } else if recovered {
        progress(
            window,
            "已恢复上一版本",
            100,
            "warning",
            "上一版本已恢复运行",
        );
    } else {
        progress(
            window,
            "部署未完成",
            100,
            "error",
            "请查看事务日志与快照目录",
        );
    }
    Ok(VersionDeployResult {
        success,
        recovered,
        recovery_failed,
        stage,
        version: bundle.version().to_string(),
        previous_version: plan.previous_version,
        transaction_id: transaction.id.clone(),
        snapshot_dir: transaction.snapshot_dir.clone(),
        error: transaction.error.clone(),
        output,
        duration_secs: started.elapsed().as_secs(),
        status: Some(transaction),
    })
}

struct RemotePlan {
    previous_version: String,
    had_deployment: bool,
    old_compose: String,
    new_compose: String,
    project_name: String,
    services: Vec<String>,
    old_services: Vec<String>,
    frontend_service: String,
    frontend_port: Option<u16>,
    old_frontend_port: Option<u16>,
    volumes: Vec<RemoteVolumePlan>,
    old_images: Vec<ImageSnapshot>,
    new_images: BTreeMap<String, ImageSnapshot>,
    image_override: String,
    old_image_override: String,
    managed_paths: Vec<String>,
    retired_paths: Vec<String>,
    required_backup_bytes: u64,
    baseline: String,
    env_content: String,
}

#[derive(Clone)]
struct RemoteVolumePlan {
    logical: String,
    actual: String,
    image: String,
    exists: bool,
    bytes: u64,
    required: bool,
}

fn remote_preflight(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    bundle: &Bundle,
    install: &str,
) -> Result<RemotePlan, String> {
    let deploy_dir = remote_join(install, &bundle.deploy_source_rel());
    let record = remote_read_record(host, port, user, key, install)?;
    if record.as_ref().is_some_and(|record| {
        record.product != bundle.product() || !matches!(record.schema_version, 1 | 2)
    }) {
        return Err("目标目录的旧部署记录与当前产品或记录版本不匹配，已停止部署".to_string());
    }
    let previous_version = record
        .as_ref()
        .map(|record| record.version.clone())
        .unwrap_or_default();
    let old_compose =
        remote_find_compose(host, port, user, key, install, &deploy_dir, record.as_ref())?;
    let new_compose_artifact = bundle
        .artifact_for_source(bundle.effective_compose())
        .ok_or_else(|| "交付包未声明 Compose 清单".to_string())?;
    let new_compose_target = match new_compose_artifact.root {
        ArtifactRoot::Install => remote_join(install, new_compose_artifact.path.trim()),
        ArtifactRoot::Deploy => remote_join(&deploy_dir, new_compose_artifact.path.trim()),
    };
    let new_compose_managed = new_compose_target
        .strip_prefix(&format!("{}/", install.trim_end_matches('/')))
        .unwrap_or(new_compose_artifact.path.trim())
        .to_string();
    let new_compose = new_compose_target
        .strip_prefix(&format!("{}/", deploy_dir.trim_end_matches('/')))
        .unwrap_or(new_compose_artifact.path.trim())
        .to_string();
    let preflight_id = format!(
        "preflight-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or(0),
        std::process::id()
    );
    let remote_temp = format!("/tmp/deploy-desktop-{preflight_id}");
    ensure_remote_dir(host, port, user, key, &remote_temp)?;
    let cleanup = RemoteCleanup {
        host: host.to_string(),
        port,
        user: user.to_string(),
        key: key.map(str::to_string),
        path: remote_temp.clone(),
    };
    let env_content = crate::commands::projects::read_active_project_env()?.unwrap_or_default();
    let env_file = local_temp_file("env", env_content.as_bytes())?;
    let compose_source = bundle.root.join(bundle.effective_compose());
    if !env_content.is_empty() {
        crate::commands::remote_ssh::ensure_known_host(host, port)?;
    }
    upload_file_with_progress(
        host,
        port,
        user,
        key,
        &compose_source,
        &format!("{remote_temp}/compose"),
        None,
    )?;
    upload_file_with_progress(
        host,
        port,
        user,
        key,
        env_file.path(),
        &format!("{remote_temp}/env"),
        None,
    )?;
    let config_script = format!(
        r#"set -eo pipefail
{baseline_script}
TMP={tmp}
DEPLOY={deploy}
docker compose version >/dev/null 2>&1 || {{ echo '目标机 Docker Compose 不支持完整版本部署' >&2; exit 3; }}
ENV_ARGS=()
[ -f "$DEPLOY/.env" ] && ENV_ARGS+=(--env-file "$DEPLOY/.env")
[ -s "$TMP/env" ] && ENV_ARGS+=(--env-file "$TMP/env")
[ -f "$DEPLOY/.env.local" ] && ENV_ARGS+=(--env-file "$DEPLOY/.env.local")
docker compose --project-directory "$DEPLOY" ${{ENV_ARGS[@]+"${{ENV_ARGS[@]}}"}} -f "$TMP/compose" config --format json > "$TMP/incoming.json"
echo INCOMING_CONFIG_B64=$(base64 < "$TMP/incoming.json" | tr -d '\n')
OLD={old}
printf 'BASELINE=%s\n' "$(deployment_baseline {install} "$DEPLOY" "$OLD")"
if [ -n "$OLD" ] && [ -f "$DEPLOY/$OLD" ]; then
  ENV_ARGS=()
  [ -f "$DEPLOY/.env" ] && ENV_ARGS+=(--env-file "$DEPLOY/.env")
  [ -f "$DEPLOY/.env.local" ] && ENV_ARGS+=(--env-file "$DEPLOY/.env.local")
  docker compose --project-directory "$DEPLOY" ${{ENV_ARGS[@]+"${{ENV_ARGS[@]}}"}} -f "$DEPLOY/$OLD" config --format json > "$TMP/current.json"
  echo HAS_DEPLOYMENT=yes
  echo CURRENT_CONFIG_B64=$(base64 < "$TMP/current.json" | tr -d '\n')
else
  echo HAS_DEPLOYMENT=no
fi
"#,
        tmp = shell_quote(&remote_temp),
        deploy = shell_quote(&deploy_dir),
        old = shell_quote(&old_compose),
        install = shell_quote(install),
        baseline_script = include_str!("../../../../shell/deployment-baseline.sh")
    );
    let output = run_ssh_script_with_timeout(
        host,
        port,
        user,
        key,
        &config_script,
        Duration::from_secs(300),
    )?;
    drop(env_file);
    if !output.status.success() {
        return Err(format!(
            "目标机预检失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let baseline = raw
        .lines()
        .find_map(|line| line.strip_prefix("BASELINE="))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "目标机预检缺少部署基线摘要".to_string())?
        .to_string();
    let has_deployment = raw
        .lines()
        .find_map(|line| line.strip_prefix("HAS_DEPLOYMENT="))
        .map(|value| value.trim() == "yes")
        .unwrap_or(false);
    let incoming: Value = parse_remote_json(&raw, "INCOMING_CONFIG_B64")?;
    let current = if has_deployment {
        Some(parse_remote_json(&raw, "CURRENT_CONFIG_B64")?)
    } else {
        None
    };
    let project_name = incoming
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "新版本 Compose 清单缺少项目名称".to_string())?;
    let project_containers = run_ssh_script(
        host,
        port,
        user,
        key,
        &format!(
            "test -n \"$(docker ps -a -q --filter {})\"",
            shell_quote(&format!("label=com.docker.compose.project={project_name}"))
        ),
    )?
    .status
    .success();
    if !has_deployment || project_containers {
        crate::commands::version_deploy::validate_new_deployment_isolation(
            &incoming,
            project_containers,
        )?;
    }
    if let Some(current) = &current {
        let current_name = current
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if current_name != project_name {
            return Err(format!(
                "新旧版本 Compose 项目名称不同（{current_name} -> {project_name}），已阻止部署"
            ));
        }
    }
    let services = active_services(&incoming);
    crate::commands::version_deploy::verify_declared_image_references(bundle, &incoming)?;
    let unknown: Vec<_> = services
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
    if !unknown.is_empty() {
        return Err("Compose 包含未声明角色的服务，请重新构建制品".to_string());
    }
    let old_services = current.as_ref().map(active_services).unwrap_or_default();
    let missing: Vec<&str> = bundle
        .descriptor
        .runtime
        .services
        .iter()
        .map(|service| service.key.as_str())
        .filter(|service| !services.iter().any(|item| item == service))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "交付包声明的服务未出现在 Compose 配置：{}",
            missing.join("、")
        ));
    }
    let image_refs = unique_image_refs(&incoming);
    let old_image_refs = current.as_ref().map(unique_image_refs).unwrap_or_default();
    let mut volume_plans =
        remote_volume_plans(bundle, &incoming, current.as_ref(), has_deployment)?;
    let resources = remote_preflight_resources(
        host,
        port,
        user,
        key,
        install,
        &image_refs,
        &old_image_refs,
        &volume_plans,
        &project_name,
    )?;
    volume_plans = resources.volumes;
    let mut required = volume_plans
        .iter()
        .map(|volume| volume.bytes)
        .sum::<u64>()
        .saturating_add(
            bundle
                .descriptor
                .files
                .iter()
                .map(|file| file.size)
                .sum::<u64>()
                .saturating_mul(2),
        )
        .saturating_add(512 * 1024 * 1024);
    let free = resources.free_kb.saturating_mul(1024);
    if free < required {
        return Err(format!(
            "目标机空间不足：需要约 {:.1} GB，可用 {:.1} GB",
            required as f64 / 1024.0 / 1024.0 / 1024.0,
            free as f64 / 1024.0 / 1024.0 / 1024.0
        ));
    }
    let mut managed_paths: BTreeSet<String> = bundle
        .resolve_artifacts(
            &bundle.select_components(&default_components(bundle))?,
            Path::new(install),
            Path::new(&deploy_dir),
        )?
        .iter()
        .map(|artifact| {
            artifact
                .target
                .strip_prefix(Path::new(install))
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default()
        })
        .filter(|path| !path.is_empty())
        .collect();
    let mut new_managed_paths = managed_paths.clone();
    let mut retired_paths = BTreeSet::new();
    if let Some(record) = &record {
        for artifact in &record.artifacts {
            let relative = match artifact.root.as_str() {
                "install" => artifact.path.clone(),
                "deploy" => remote_join(&bundle.deploy_source_rel(), &artifact.path),
                _ => continue,
            };
            managed_paths.insert(relative.clone());
            if !new_managed_paths.contains(&relative) {
                retired_paths.insert(relative);
            }
        }
    }
    let env_rel = remote_join(&bundle.deploy_source_rel(), ".env");
    managed_paths.insert(env_rel.clone());
    new_managed_paths.insert(env_rel);
    let deployed_rel = remote_join("", &format!("{DEPLOYED_RECORD_DIR}/{DEPLOYED_RECORD_FILE}"));
    managed_paths.insert(deployed_rel.clone());
    new_managed_paths.insert(deployed_rel);
    if !old_compose.is_empty() {
        managed_paths.insert(remote_join(&bundle.deploy_source_rel(), &old_compose));
        if old_compose != new_compose {
            retired_paths.insert(remote_join(&bundle.deploy_source_rel(), &old_compose));
        }
    }
    managed_paths.insert(new_compose_managed.clone());
    new_managed_paths.insert(new_compose_managed);
    retired_paths.retain(|path| !new_managed_paths.contains(path));
    let mut retired_paths: Vec<String> = retired_paths.into_iter().collect();
    retired_paths.sort();
    retired_paths = minimal_retired_paths(retired_paths);
    crate::commands::version_deploy::validate_deployment_mounts(
        bundle,
        install,
        &managed_paths,
        &incoming,
    )?;
    if let Some(current) = &current {
        crate::commands::version_deploy::validate_deployment_mounts(
            bundle,
            install,
            &managed_paths,
            current,
        )?;
    }
    ensure_preserved_paths_untouched(&managed_paths, bundle)?;
    ensure_preserved_paths_untouched(&retired_paths.iter().cloned().collect(), bundle)?;
    let size_arguments = std::iter::once(install.to_string())
        .chain(managed_paths.iter().cloned())
        .map(|value| shell_quote(&value))
        .collect::<Vec<_>>()
        .join(" ");
    let size_output = run_ssh_script_with_timeout(
        host,
        port,
        user,
        key,
        &format!(
            "set -eo pipefail\n{}\ndeployment_managed_bytes {size_arguments}\n",
            include_str!("../../../../shell/deployment-baseline.sh")
        ),
        Duration::from_secs(300),
    )?;
    if !size_output.status.success() {
        return Err(format!(
            "无法安全建立文件快照：{}",
            String::from_utf8_lossy(&size_output.stderr).trim()
        ));
    }
    let file_bytes = String::from_utf8_lossy(&size_output.stdout)
        .trim()
        .parse::<u64>()
        .map_err(|_| "无法估算目标文件快照空间".to_string())?;
    required = required.saturating_add(file_bytes);
    if free < required {
        return Err(format!(
            "目标机空间不足：含文件快照与暂存内容需要约 {:.1} GB",
            required as f64 / 1024.0 / 1024.0 / 1024.0
        ));
    }
    for volume in &mut volume_plans {
        if !has_deployment && volume.exists {
            return Err(format!(
                "首次部署的目标已存在数据卷 {}，为避免误删数据已停止部署",
                volume.actual
            ));
        }
        if has_deployment && volume.required && !volume.exists {
            return Err(format!(
                "现有部署缺少必需数据卷 {}，无法建立可恢复快照",
                volume.actual
            ));
        }
    }
    let image_override = crate::commands::version_deploy::image_override_document(
        &incoming,
        &resources.new_images.values().cloned().collect::<Vec<_>>(),
    )?
    .unwrap_or_default();
    let old_image_override = current
        .as_ref()
        .map(|config| {
            crate::commands::version_deploy::image_override_document(config, &resources.old_images)
        })
        .transpose()?
        .flatten()
        .unwrap_or_default();
    if let Some(frontend) = bundle
        .descriptor
        .runtime
        .services
        .iter()
        .find(|service| service.key.eq_ignore_ascii_case("frontend"))
    {
        if current.as_ref().is_some_and(|config| {
            config
                .get("services")
                .and_then(|services| services.get(&frontend.key))
                .is_some()
                && crate::commands::version_deploy::published_frontend_port(
                    config,
                    &frontend.key,
                    frontend.port,
                )
                .is_none()
        }) {
            return Err(
                "无法确定上一版本网页的 HTTP 端口，请确认旧版本的网页端口声明后再部署".to_string(),
            );
        }
    }
    drop(cleanup);
    Ok(RemotePlan {
        previous_version,
        had_deployment: has_deployment,
        old_compose,
        new_compose,
        project_name,
        services,
        old_services,
        frontend_service: bundle
            .descriptor
            .runtime
            .services
            .iter()
            .find(|service| service.key.eq_ignore_ascii_case("frontend"))
            .map(|service| service.key.clone())
            .unwrap_or_default(),
        frontend_port: bundle
            .descriptor
            .runtime
            .services
            .iter()
            .find(|service| service.key.eq_ignore_ascii_case("frontend"))
            .and_then(|service| service.port),
        old_frontend_port: current.as_ref().and_then(|config| {
            let frontend = bundle
                .descriptor
                .runtime
                .services
                .iter()
                .find(|service| service.key.eq_ignore_ascii_case("frontend"))?;
            crate::commands::version_deploy::published_frontend_port(
                config,
                &frontend.key,
                frontend.port,
            )
        }),
        volumes: volume_plans,
        old_images: resources.old_images,
        new_images: resources.new_images,
        image_override,
        old_image_override,
        managed_paths: managed_paths.into_iter().collect(),
        retired_paths,
        required_backup_bytes: required,
        baseline,
        env_content,
    })
}

fn minimal_retired_paths(paths: Vec<String>) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for path in paths {
        if result
            .iter()
            .any(|parent| path == *parent || path.starts_with(&format!("{parent}/")))
        {
            continue;
        }
        result.push(path);
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

fn remote_read_record(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install: &str,
) -> Result<Option<DeployedRecord>, String> {
    let path = format!(
        "{}/{}/{}",
        install.trim_end_matches('/'),
        DEPLOYED_RECORD_DIR,
        DEPLOYED_RECORD_FILE
    );
    let output = run_ssh_script(
        host,
        port,
        user,
        key,
        &format!(
            "set -eo pipefail\nif [ -e {path} ]; then [ -f {path} ] && [ -s {path} ] || exit 4; base64 < {path} | tr -d '\\n'; fi",
            path = shell_quote(&path)
        ),
    )?;
    if !output.status.success() {
        return Err("上一版本部署记录不可读，请确认旧部署状态后再部署".to_string());
    }
    let encoded = String::from_utf8_lossy(&output.stdout);
    if encoded.trim().is_empty() {
        return Ok(None);
    }
    let bytes = decode_base64(encoded.trim())?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "上一版本部署记录格式无效，请确认旧部署状态后再部署".to_string())
}

fn remote_find_compose(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install: &str,
    deploy_dir: &str,
    record: Option<&DeployedRecord>,
) -> Result<String, String> {
    let stored_effective = record
        .map(|record| record.effective_compose.as_str())
        .unwrap_or_default();
    if !stored_effective.is_empty() {
        let path = stored_effective_path(install, stored_effective)?;
        let marker = inspect_remote_compose_path(
            host, port, user, key, deploy_dir, &path,
        )?;
        return resolve_stored_effective_compose(
            install,
            deploy_dir,
            stored_effective,
            &marker,
        );
    }
    let active_output = run_ssh_script(
        host, port, user, key,
        &format!(
            "docker ps -a --filter {} --format '{{{{.Label \"com.docker.compose.project.config_files\"}}}}'",
            shell_quote(&format!("label=com.docker.compose.project.working_dir={deploy_dir}"))
        ),
    )?;
    if !active_output.status.success() {
        return Err("无法检查上一版本的实际 Compose 清单".to_string());
    }
    let active_text = String::from_utf8_lossy(&active_output.stdout);
    for path in active_text
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        if !path.starts_with(&format!("{deploy_dir}/"))
            && !path.starts_with(&format!("{install}/.deploy-desktop/"))
        {
            return Err("上一版本引用部署目录外的 Compose 清单，本期无法安全恢复".to_string());
        }
    }
    let actual: BTreeSet<_> = active_text
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter_map(|path| path.strip_prefix(&format!("{deploy_dir}/")))
        .map(str::to_string)
        .collect();
    if actual.len() > 1 {
        return Err(
            "上一版本合并使用多份 Compose 清单，本期无法安全恢复，请先确认生效配置".to_string(),
        );
    }
    if let Some(relative) = actual.into_iter().next() {
        let path = remote_join(deploy_dir, &relative);
        let marker =
            inspect_remote_compose_path(host, port, user, key, deploy_dir, &path)?;
        return resolve_stored_effective_compose(install, deploy_dir, &path, &marker);
    }
    let mut candidates: Vec<String> = Vec::new();
    if let Some(record) = record {
        for compose in &record.compose_files {
            let path = Path::new(&compose.path);
            let relative = path
                .strip_prefix(deploy_dir)
                .map(|value| value.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| {
                    if path.is_absolute() {
                        compose.path.clone()
                    } else {
                        compose
                            .path
                            .strip_prefix(&format!(
                                "{}/",
                                deploy_dir
                                    .strip_prefix(&format!("{install}/"))
                                    .unwrap_or(deploy_dir)
                            ))
                            .unwrap_or(&compose.path)
                            .to_string()
                    }
                });
            candidates.push(relative);
        }
        for artifact in &record.artifacts {
            if artifact.path.ends_with(".yml") || artifact.path.ends_with(".yaml") {
                candidates.push(match artifact.root.as_str() {
                    "deploy" => artifact.path.clone(),
                    "install" => remote_join(install, &artifact.path),
                    _ => continue,
                });
            }
        }
    }
    for name in [
        "docker-compose.lan.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yaml",
        "compose.yml",
    ] {
        candidates.push(name.to_string());
    }
    candidates.sort();
    candidates.dedup();
    let checks = candidates
        .iter()
        .map(|candidate| {
            format!(
                "if [ -f {full} ]; then printf '%s\\n' {label}; fi",
                full = shell_quote(&if candidate.starts_with('/') {
                    candidate.clone()
                } else {
                    remote_join(deploy_dir, candidate)
                }),
                label = shell_quote(candidate)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let output = run_ssh_script(
        host,
        port,
        user,
        key,
        &format!("set -e\n{checks}\nexit 0\n"),
    )?;
    if !output.status.success() {
        return Err("读取目标机 Compose 清单失败".to_string());
    }
    let found: Vec<_> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    if found.len() > 1 {
        return Err(
            "旧部署未记录实际生效的 Compose，且存在多份清单；请先确认旧版本的生效清单".to_string(),
        );
    }
    let selected = found.into_iter().next().unwrap_or_default();
    if selected.starts_with('/') {
        return Err("上一版本 Compose 位于其他目录，本期不支持迁移部署目录".to_string());
    }
    if selected.is_empty() && record.is_some() {
        return Err("旧部署记录存在，但找不到上一版本 Compose 清单，已停止部署".to_string());
    }
    if selected.is_empty() {
        return Ok(selected);
    }
    let path = remote_join(deploy_dir, &selected);
    let marker = inspect_remote_compose_path(host, port, user, key, deploy_dir, &path)?;
    resolve_stored_effective_compose(install, deploy_dir, &path, &marker)
}

fn inspect_remote_compose_path(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    deploy_dir: &str,
    path: &str,
) -> Result<String, String> {
    let output = run_ssh_script(
        host,
        port,
        user,
        key,
        &format!(
            r#"if [ ! -e {path} ]; then
  printf 'MISSING\n'
elif [ ! -f {path} ]; then
  printf 'INVALID\n'
else
  base=$(readlink -f -- {deploy} 2>/dev/null) || {{ printf 'INVALID\n'; exit 0; }}
  target=$(readlink -f -- {path} 2>/dev/null) || {{ printf 'INVALID\n'; exit 0; }}
  case "$target" in
    "$base"/*) printf 'FOUND\n' ;;
    *) printf 'OUTSIDE\n' ;;
  esac
fi"#,
            deploy = shell_quote(deploy_dir),
            path = shell_quote(path)
        ),
    )?;
    if !output.status.success() {
        return Err("无法检查上一版本 Compose 清单路径".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn stored_effective_path(install: &str, stored_effective: &str) -> Result<String, String> {
    if stored_effective.contains(['\\', '\0', '\n', '\r'])
        || Path::new(stored_effective)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("上一版本 Compose 路径包含无效跳转，已停止部署".to_string());
    }
    Ok(if stored_effective.starts_with('/') {
        stored_effective.to_string()
    } else {
        remote_join(install, stored_effective)
    })
}

fn resolve_stored_effective_compose(
    install: &str,
    deploy_dir: &str,
    stored_effective: &str,
    marker: &str,
) -> Result<String, String> {
    let path = stored_effective_path(install, stored_effective)?;
    let relative = path
        .strip_prefix(&format!("{}/", deploy_dir.trim_end_matches('/')))
        .ok_or_else(|| "上一版本 Compose 位于其他目录，本期不支持迁移部署目录".to_string())?;
    match marker.trim() {
        "FOUND" => Ok(relative.to_string()),
        "MISSING" => Err("上一版本的生效 Compose 清单不存在，已停止部署".to_string()),
        "OUTSIDE" => {
            Err("上一版本 Compose 位于其他目录，本期不支持迁移部署目录".to_string())
        }
        _ => Err("无法确认上一版本 Compose 清单路径，已停止部署".to_string()),
    }
}

fn remote_preflight_resources(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install: &str,
    image_refs: &[String],
    old_image_refs: &[String],
    volumes: &[RemoteVolumePlan],
    project_name: &str,
) -> Result<RemoteResources, String> {
    let bindings = crate::commands::deployment_image_bindings::remote_bindings(
        host, port, user, key, install,
    )?;
    let identity = |reference: &str| {
        bindings
            .iter()
            .find(|binding| binding.reference == reference)
            .map(|binding| binding.image_id.clone())
            .unwrap_or_else(|| reference.to_string())
    };
    let image_checks = image_refs
        .iter()
        .enumerate()
        .map(|(index, reference)| {
            format!(
                "if docker image inspect --format 'NEW_IMAGE|{index}|{{{{.Id}}}}|{{{{.Os}}}}/{{{{.Architecture}}}}' -- {reference} 2>/dev/null; then :; else echo '缺少新版本镜像 {reference}' >&2; exit 4; fi",
                reference = shell_quote(&identity(reference))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let old_image_checks = old_image_refs
        .iter()
        .enumerate()
        .map(|(index, reference)| {
            format!(
                "if ! docker image inspect --format 'OLD_IMAGE|{index}|{{{{.Id}}}}|{{{{.Os}}}}/{{{{.Architecture}}}}' -- {reference} 2>/dev/null; then echo '缺少上一版本镜像 {reference}' >&2; exit 5; fi",
                reference = shell_quote(&identity(reference))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let volume_checks = volumes
        .iter()
        .enumerate()
        .map(|(index, volume)| {
            format!(
                r#"if docker volume inspect -- {actual} >/dev/null 2>&1; then
  exists=yes
  owner=$(docker volume inspect --format '{{{{index .Labels "com.docker.compose.project"}}}}|{{{{index .Labels "com.docker.compose.volume"}}}}' -- {actual})
  [ "$owner" = {owner} ] || {{ echo '数据卷 Compose 归属标签不一致，已阻止部署' >&2; exit 6; }}
  bytes=$(docker run --rm --network none -v {actual}:/data:ro --entrypoint /bin/sh {image} -c "du -sk /data 2>/dev/null | awk '{{print \$1}}'" 2>/dev/null | tr -d '\r')
  bytes=$(( ${{bytes:-0}} * 1024 ))
else
  exists=no
  bytes=0
fi
if ! docker run --rm --network none --entrypoint /bin/sh {image} -c 'command -v tar >/dev/null && command -v gzip >/dev/null' >/dev/null 2>&1; then
  echo '备份镜像 {image} 缺少 tar 或 gzip' >&2
  exit 6
fi
echo "VOLUME|{index}|$exists|$bytes"
"#,
                actual = shell_quote(&volume.actual),
                image = shell_quote(&identity(&volume.image)),
                owner = shell_quote(&format!("{project_name}|{}", volume.logical))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let script = format!(
        r#"set -e
docker info >/dev/null 2>&1 || {{ echo 'Docker 未运行或当前用户无权访问' >&2; exit 2; }}
command -v tar >/dev/null 2>&1 || {{ echo '目标机缺少 tar' >&2; exit 2; }}
command -v gzip >/dev/null 2>&1 || {{ echo '目标机缺少 gzip' >&2; exit 2; }}
command -v sha256sum >/dev/null 2>&1 || {{ echo '目标机缺少 sha256sum' >&2; exit 2; }}
command -v flock >/dev/null 2>&1 || {{ echo '目标机缺少 flock，无法获取部署锁' >&2; exit 2; }}
{image_checks}
{old_image_checks}
{volume_checks}
SPACE_PATH={install}
[ -e "$SPACE_PATH" ] || SPACE_PATH=$(dirname "$SPACE_PATH")
echo "FREE_KB=$(df -Pk "$SPACE_PATH" | awk 'NR==2{{print $4}}')"
"#,
        image_checks = image_checks,
        old_image_checks = old_image_checks,
        volume_checks = volume_checks,
        install = shell_quote(install)
    );
    let output =
        run_ssh_script_with_timeout(host, port, user, key, &script, Duration::from_secs(600))?;
    if !output.status.success() {
        return Err(format!(
            "目标机资源预检失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let free_kb = raw
        .lines()
        .find_map(|line| line.strip_prefix("FREE_KB="))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let mut new_images = BTreeMap::new();
    for line in raw.lines().filter(|line| line.starts_with("NEW_IMAGE|")) {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() != 4 {
            continue;
        }
        let index = parts[1].parse::<usize>().unwrap_or(usize::MAX);
        let Some(reference) = image_refs.get(index) else {
            continue;
        };
        new_images.insert(
            reference.clone(),
            ImageSnapshot {
                reference: reference.clone(),
                image_id: parts[2].to_string(),
                platform: parts[3].to_string(),
            },
        );
    }
    if new_images.len() != image_refs.len() {
        return Err("新版本镜像状态核对不完整".to_string());
    }
    let mut old_images = Vec::new();
    for line in raw.lines().filter(|line| line.starts_with("OLD_IMAGE|")) {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() != 4 {
            continue;
        }
        let index = parts[1].parse::<usize>().unwrap_or(usize::MAX);
        let Some(reference) = old_image_refs.get(index) else {
            continue;
        };
        old_images.push(ImageSnapshot {
            reference: reference.clone(),
            image_id: parts[2].to_string(),
            platform: parts[3].to_string(),
        });
    }
    if old_images.len() != old_image_refs.len() {
        return Err("上一版本镜像状态核对不完整".to_string());
    }
    let mut checked_volumes = volumes.to_vec();
    for line in raw.lines().filter(|line| line.starts_with("VOLUME|")) {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() != 4 {
            continue;
        }
        let index = parts[1].parse::<usize>().unwrap_or(usize::MAX);
        if let Some(volume) = checked_volumes.get_mut(index) {
            volume.exists = parts[2] == "yes";
            volume.bytes = parts[3].parse::<u64>().unwrap_or(0);
            volume.image = new_images
                .get(&volume.image)
                .map(|image| image.image_id.clone())
                .unwrap_or_else(|| identity(&volume.image));
        }
    }
    Ok(RemoteResources {
        free_kb,
        new_images,
        old_images,
        volumes: checked_volumes,
    })
}

#[derive(Clone)]
struct RemoteResources {
    free_kb: u64,
    new_images: BTreeMap<String, ImageSnapshot>,
    old_images: Vec<ImageSnapshot>,
    volumes: Vec<RemoteVolumePlan>,
}

fn remote_volume_plans(
    bundle: &Bundle,
    incoming: &Value,
    current: Option<&Value>,
    had_deployment: bool,
) -> Result<Vec<RemoteVolumePlan>, String> {
    let data = bundle
        .descriptor
        .runtime
        .data
        .as_ref()
        .ok_or_else(|| "交付包缺少数据卷声明".to_string())?;
    let mut result = Vec::new();
    for volume in &data.volumes {
        let actual = volume_name(incoming, &volume.name)?;
        if !volume_mounted_by(incoming, &volume.service, &volume.name) {
            return Err(format!(
                "数据卷 {} 未挂载到声明的服务 {}，请修正交付包",
                volume.name, volume.service
            ));
        }
        if let Some(current) = current {
            if volume_name(current, &volume.name)? != actual {
                return Err(format!("新旧版本数据卷 {} 的实际名称不一致", volume.name));
            }
        }
        let image = bundle
            .descriptor
            .images
            .iter()
            .find(|image| image.name == volume.image)
            .map(|image| image.reference.clone())
            .ok_or_else(|| format!("数据卷 {} 的备份镜像未声明", volume.name))?;
        result.push(RemoteVolumePlan {
            logical: volume.name.clone(),
            actual,
            image,
            exists: false,
            bytes: 0,
            required: volume.required,
        });
    }
    for service in active_service_configs(incoming) {
        if let Some(items) = service.get("volumes").and_then(Value::as_array) {
            for item in items {
                if item.get("type").and_then(Value::as_str) != Some("volume") {
                    continue;
                }
                let source = item
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !result.iter().any(|volume| volume.logical == source) {
                    return Err(format!("服务使用了未声明的数据卷 {source}"));
                }
            }
        }
    }
    let _ = had_deployment;
    Ok(result)
}

fn volume_mounted_by(config: &Value, service: &str, logical: &str) -> bool {
    config
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
        .unwrap_or(false)
}

fn volume_name(config: &Value, logical: &str) -> Result<String, String> {
    let volume = config
        .get("volumes")
        .and_then(|value| value.get(logical))
        .and_then(Value::as_object)
        .ok_or_else(|| format!("Compose 未声明数据卷 {logical}"))?;
    if volume.get("external").and_then(Value::as_bool) == Some(true) {
        return Err(format!("数据卷 {logical} 为 external，无法自动备份"));
    }
    volume
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("无法解析数据卷 {logical} 的实际名称"))
}

fn unique_image_refs(config: &Value) -> Vec<String> {
    let mut result = Vec::new();
    for service in active_service_configs(config) {
        if let Some(reference) = service
            .get("image")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            if !result.iter().any(|item| item == reference) {
                result.push(reference.to_string());
            }
        }
    }
    result
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

fn remote_deployed_record(
    bundle: &Bundle,
    install: &str,
    deploy_dir: &str,
    images: &BTreeMap<String, ImageSnapshot>,
    volumes: &[RemoteVolumePlan],
    transaction_id: &str,
    env_content: &str,
) -> Result<DeployedRecord, String> {
    let mut record = bundle.deployed_record(
        &crate::commands::projects::active_project()
            .map(|project| project.id)
            .unwrap_or_default(),
    );
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
            ArtifactRoot::Install => install.to_string(),
            ArtifactRoot::Deploy => deploy_dir.to_string(),
        };
        compose_files.push(DeployedComposeFile {
            path: remote_join(&base, artifact.path.trim()),
            sha256: bundle
                .descriptor
                .files
                .iter()
                .find(|file| file.path.trim() == source)
                .map(|file| file.sha256.to_ascii_lowercase())
                .unwrap_or_default(),
        });
    }
    record.compose_files = compose_files;
    record.env_sha256 = sha256_hex(env_content.as_bytes());
    record.data_volumes = volumes
        .iter()
        .map(|volume| volume.logical.clone())
        .collect();
    record.transaction_id = Some(transaction_id.to_string());
    Ok(record)
}

fn verify_remote_service_declarations(bundle: &Bundle) -> Result<Option<String>, String> {
    let content = fs::read_to_string(bundle.root.join(bundle.effective_compose()))
        .map_err(|_| "无法读取交付包 Compose 清单".to_string())?;
    let available: BTreeSet<String> = crate::core::compose::service_names(&content)
        .map_err(|_| "交付包 Compose 清单格式错误".to_string())?
        .into_iter()
        .collect();
    let missing: Vec<&str> = bundle
        .descriptor
        .runtime
        .services
        .iter()
        .map(|service| service.key.as_str())
        .filter(|service| !available.contains(*service))
        .collect();
    if missing.is_empty() {
        Ok(None)
    } else {
        Ok(Some(format!(
            "交付包声明的服务未出现在 Compose 清单：{}",
            missing.join("、")
        )))
    }
}

fn launch_worker(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    txn: &str,
    mode: &str,
    args: &[&str],
) -> Result<(), String> {
    let mut values = vec![txn.to_string(), mode.to_string()];
    values.extend(args.iter().map(|value| value.to_string()));
    let arguments = values.iter().map(String::as_str).collect::<Vec<_>>();
    let script = crate::core::shell::script(
        include_str!("../../../../shell/version-deploy-launch.sh"),
        &arguments,
    )?;
    let output =
        run_ssh_script_with_timeout(host, port, user, key, &script, Duration::from_secs(60))?;
    if !output.status.success() {
        return Err(format!(
            "无法启动目标机部署任务：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn poll_remote_transaction(
    window: &tauri::Window,
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install: &str,
    timeout: Duration,
) -> Result<DeploymentTransaction, String> {
    let deadline = Instant::now() + timeout;
    let mut last_percent = 50;
    loop {
        let transaction = inspect_remote_transaction(host, port, user, key, install)?;
        if let Some(transaction) = transaction {
            let percent = match transaction.phase.as_str() {
                "preparing" => 55,
                "snapshotting_files" => 60,
                "snapshotting_data" => 67,
                "applying" => 78,
                "starting" => 88,
                "recovering" => 92,
                "committed" => 100,
                "recovered" => 100,
                "recovery_failed" => 100,
                _ => last_percent,
            };
            if percent > last_percent {
                last_percent = percent;
                progress(
                    window,
                    phase_label(&transaction.phase),
                    percent,
                    "running",
                    "目标机后台部署任务执行中",
                );
            }
            if matches!(
                transaction.state.as_str(),
                "committed" | "recovered" | "recovery_failed" | "failed"
            ) {
                return Ok(transaction);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "等待目标机部署事务超时。任务可能仍在运行，请重新连接后检查事务状态"
            ));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn phase_label(phase: &str) -> &'static str {
    match phase {
        "snapshotting_files" => "保存部署文件",
        "snapshotting_data" => "备份数据卷",
        "applying" => "写入完整版本",
        "starting" => "启动并验证",
        "recovering" => "恢复上一版本",
        "committed" => "部署完成",
        "recovered" => "已恢复上一版本",
        "recovery_failed" => "自动恢复失败",
        _ => "部署事务执行中",
    }
}

fn read_remote_tail(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    txn: &str,
) -> Result<String, String> {
    let output = run_ssh_script(
        host,
        port,
        user,
        key,
        &format!(
            "if [ -f {log} ]; then tail -n 160 {log}; fi",
            log = shell_quote(&format!("{txn}/worker.log"))
        ),
    )?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Ok(String::new())
    }
}

fn remote_deploy_lock_active(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install: &str,
) -> Result<bool, String> {
    let lock = format!(
        "{}/.deploy-desktop/deploy.lock",
        install.trim_end_matches('/')
    );
    let output = run_ssh_script(
        host,
        port,
        user,
        key,
        &format!(
            r#"command -v flock >/dev/null 2>&1 || exit 2
if [ -f {lock} ] && ! flock -n {lock} -c true; then
  echo ACTIVE
fi
"#,
            lock = shell_quote(&lock)
        ),
    )?;
    if !output.status.success() {
        return Err("无法检查目标机部署锁，请确认 flock 可用".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).contains("ACTIVE"))
}

fn write_remote_text(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    path: &str,
    content: &str,
    private: bool,
) -> Result<(), String> {
    let encoded = crate::commands::remote_ssh::base64_encode(content)?;
    let parent = Path::new(path)
        .parent()
        .ok_or_else(|| "目标机文件路径无效".to_string())?
        .to_string_lossy();
    let mode = if private { "600" } else { "644" };
    let run_script = if private {
        crate::commands::remote_ssh::run_ssh_script_strict_with_timeout
    } else {
        run_ssh_script_with_timeout
    };
    let script = format!(
        "set -e\nmkdir -p {parent}\numask 077\ntmp=$(mktemp {template})\ntrap 'rm -f \"$tmp\"' EXIT\nprintf '%s' {encoded} | base64 -d > \"$tmp\"\nchmod {mode} \"$tmp\"\nmv -f \"$tmp\" {path}\ntrap - EXIT\n",
        parent = shell_quote(&parent),
        template = shell_quote(&format!("{path}.tmp.XXXXXXXX")),
        encoded = shell_quote(&encoded),
        mode = mode,
        path = shell_quote(path)
    );
    let output = run_script(host, port, user, key, &script, Duration::from_secs(120))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "写入目标机事务文件失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn ensure_remote_dir(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    path: &str,
) -> Result<(), String> {
    let output = run_ssh_script(
        host,
        port,
        user,
        key,
        &format!("umask 077\nmkdir -p {}", shell_quote(path)),
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err("无法创建目标机部署事务目录".to_string())
    }
}

fn parse_remote_json(raw: &str, key: &str) -> Result<Value, String> {
    let encoded = raw
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .ok_or_else(|| format!("目标机预检结果缺少 {key}"))?;
    let bytes = decode_base64(encoded.trim())?;
    serde_json::from_slice(&bytes).map_err(|_| "目标机 Compose 配置无法解析".to_string())
}

fn remote_join(base: &str, child: &str) -> String {
    let base = base.trim_end_matches('/');
    let child = child.trim().trim_start_matches('/');
    if child.is_empty() {
        base.to_string()
    } else if base.is_empty() {
        child.to_string()
    } else {
        format!("{base}/{child}")
    }
}

fn local_temp_file(label: &str, content: &[u8]) -> Result<TempFile, String> {
    let path = std::env::temp_dir().join(format!(
        "deploy-desktop-{label}-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0)
    ));
    crate::commands::projects::write_private_env_bytes(&path, content)?;
    Ok(TempFile { path })
}

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct RemoteCleanup {
    host: String,
    port: u16,
    user: String,
    key: Option<String>,
    path: String,
}

impl Drop for RemoteCleanup {
    fn drop(&mut self) {
        let _ = run_ssh_script(
            &self.host,
            self.port,
            &self.user,
            self.key.as_deref(),
            &format!("rm -rf -- {}", shell_quote(&self.path)),
        );
    }
}

fn progress(window: &tauri::Window, step: &str, percent: usize, status: &str, message: &str) {
    let _ = window.emit(
        "remote-update-progress",
        super::RemoteUpdateProgress {
            step: step.to_string(),
            percent,
            status: status.to_string(),
            message: message.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::resolve_stored_effective_compose;

    #[test]
    fn stored_effective_compose_must_exist() {
        assert_eq!(
            resolve_stored_effective_compose(
                "/srv/app",
                "/srv/app/deploy",
                "deploy/docker-compose.lan.yml",
                "FOUND"
            )
            .unwrap(),
            "docker-compose.lan.yml"
        );
        let error = resolve_stored_effective_compose(
            "/srv/app",
            "/srv/app/deploy",
            "deploy/docker-compose.lan.yml",
            "MISSING",
        )
        .unwrap_err();
        assert!(error.contains("生效 Compose 清单不存在"));
    }

    #[test]
    fn stored_effective_compose_must_stay_inside_deploy_directory() {
        let error = resolve_stored_effective_compose(
            "/srv/app",
            "/srv/app/deploy",
            "/srv/other/docker-compose.yml",
            "FOUND",
        )
        .unwrap_err();
        assert!(error.contains("其他目录"));
    }

    #[test]
    fn stored_effective_compose_rejects_parent_directory_and_symlink_escape() {
        assert_eq!(
            resolve_stored_effective_compose(
                "/srv/app",
                "/srv/app/deploy",
                "/srv/app/deploy/docker-compose.yml",
                "FOUND",
            )
            .unwrap(),
            "docker-compose.yml"
        );
        let error = resolve_stored_effective_compose(
            "/srv/app",
            "/srv/app/deploy",
            "deploy/../outside/docker-compose.yml",
            "FOUND",
        )
        .unwrap_err();
        assert!(error.contains("无效跳转"));

        let error = resolve_stored_effective_compose(
            "/srv/app",
            "/srv/app/deploy",
            "/srv/app/deploy/docker-compose.yml",
            "OUTSIDE",
        )
        .unwrap_err();
        assert!(error.contains("其他目录"));
    }
}
