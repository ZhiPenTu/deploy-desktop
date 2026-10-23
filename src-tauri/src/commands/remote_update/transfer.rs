use super::scripts;
use crate::commands::remote_ssh::{find_binary, run_ssh_script_with_timeout, ssh_base_command};
use crate::core::{process, shell};
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30 * 60);

fn checked(output: Output, action: &str) -> Result<Output, String> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{action}失败（退出码 {:?}）：{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub(super) fn create_remote_backup(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install_dir: &str,
    _deploy_dir: &str,
    artifacts: &[crate::commands::bundle::ResolvedArtifact],
) -> Result<String, String> {
    let targets = artifacts
        .iter()
        .map(|artifact| artifact.target.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let script = scripts::backup(install_dir, &targets)?;
    let output = checked(
        run_ssh_script_with_timeout(host, port, user, key, &script, TRANSFER_TIMEOUT)?,
        "创建远程备份",
    )?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            line.strip_prefix("BACKUP_DIR=")
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| "备份完成但未返回备份目录".to_string())
}

fn upload_staged(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    remote: &str,
    action: &str,
    input: process::Input,
) -> Result<(), String> {
    let prepared = checked(
        run_ssh_script_with_timeout(
            host,
            port,
            user,
            key,
            &scripts::transfer("prepare", remote, "")?,
            Duration::from_secs(30),
        )?,
        "准备上传",
    )?;
    let stage = String::from_utf8_lossy(&prepared.stdout).trim().to_string();
    if !stage.starts_with(&format!("{remote}.deploy-stage.")) || stage.contains(['\r', '\n']) {
        return Err("无法确认暂存目录，已停止上传".to_string());
    }
    let result = (|| {
        let mut command = ssh_base_command(host, port, user, key);
        command.arg(shell::command(&scripts::transfer(action, remote, &stage)?));
        checked(
            process::run(
                command,
                input,
                process::Options {
                    timeout: TRANSFER_TIMEOUT,
                    ..process::Options::default()
                },
            )?,
            "上传产物",
        )?;
        // This is intentionally a separate phase: both tar and SSH must have succeeded.
        checked(
            run_ssh_script_with_timeout(
                host,
                port,
                user,
                key,
                &scripts::transfer("commit", remote, &stage)?,
                TRANSFER_TIMEOUT,
            )?,
            "替换产物",
        )?;
        Ok(())
    })();
    // Cleanup refuses to remove a previous copy when rollback could not finish.
    if let Ok(cleanup) = scripts::transfer("cleanup", remote, &stage) {
        match run_ssh_script_with_timeout(host, port, user, key, &cleanup, Duration::from_secs(30))
        {
            Ok(output) if output.status.success() => {}
            _ => log::warn!(target: "deploy", "远程暂存内容未清理，请检查部署目录"),
        }
    }
    result
}

pub(super) fn upload_file_replace(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote: &str,
) -> Result<(), String> {
    upload_file_with_progress(host, port, user, key, local, remote, None)
}

pub(crate) fn upload_file_with_progress(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote: &str,
    progress: Option<process::InputProgress>,
) -> Result<(), String> {
    if !local.is_file() {
        return Err("上传来源不是文件，请检查制品内容".to_string());
    }
    upload_staged(
        host,
        port,
        user,
        key,
        remote,
        "receive-file",
        process::Input::File {
            path: local.to_path_buf(),
            progress,
        },
    )
}

pub(super) fn upload_dir_replace(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote: &str,
) -> Result<(), String> {
    if !local.is_dir() {
        return Err("上传来源不是目录，请检查制品内容".to_string());
    }
    let mut tar = Command::new(find_binary("tar"));
    tar.args(["-czf", "-", "-C"]).arg(local).arg(".");
    upload_staged(
        host,
        port,
        user,
        key,
        remote,
        "receive-dir",
        process::Input::Producer(tar),
    )
}
