use super::scripts;
use crate::commands::remote_ssh::{run_ssh_script_with_timeout, ssh_base_command};
use crate::core::{process, shell};
use std::path::Path;
use std::process::Output;
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
    deploy_dir: &str,
    artifacts: &[crate::commands::bundle::ResolvedArtifact],
) -> Result<String, String> {
    let mut targets = artifacts
        .iter()
        .map(|artifact| artifact.target.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let deploy_dir = deploy_dir.trim().trim_end_matches('/');
    if !deploy_dir.is_empty() {
        // .env 不在产物替换列表中，但更新会重写它，备份时必须一并保留旧内容。
        targets.push(format!("{deploy_dir}/.env"));
    }
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

/// 更新结束后移除备份目录里的进行中标记，使其可以出现在清理列表。
const FINISH_BACKUP: &str = r#"INSTALL=$1
while [ "$INSTALL" != "/" ] && [ "${INSTALL%/}" != "$INSTALL" ]; do INSTALL=${INSTALL%/}; done
BACKUP=$2
case "$INSTALL" in
  /*) ;;
  *) echo '部署目录无效' >&2; exit 2 ;;
esac
case "$INSTALL" in
  *'/../'*|*/..|*'/./'*|*/.|*'//'*) echo '部署目录无效' >&2; exit 2 ;;
esac
NAME=${BACKUP##*/}
[ "$BACKUP" = "$INSTALL/backup/$NAME" ] || { echo '备份目录无效' >&2; exit 2; }
case "$NAME" in
  update-[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]-[0-9][0-9][0-9][0-9][0-9][0-9]-[[:alnum:]][[:alnum:]][[:alnum:]][[:alnum:]][[:alnum:]][[:alnum:]][[:alnum:]][[:alnum:]]) ;;
  *) echo '备份目录无效' >&2; exit 2 ;;
esac
ensure_no_symlink_path() {
  local path=$1
  while [ -n "$path" ] && [ "$path" != "/" ]; do
    [ -L "$path" ] && { echo "路径包含符号链接：$path" >&2; exit 3; }
    path=$(dirname -- "$path")
  done
}
ensure_no_symlink_path "$INSTALL"
[ -d "$BACKUP" ] || exit 0
ensure_no_symlink_path "$BACKUP"
rm -f -- "$BACKUP/.updating"
"#;

pub(super) fn finish_remote_backup(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install_dir: &str,
    backup_dir: &str,
) -> Result<(), String> {
    let install = install_dir.trim();
    let path = backup_dir.trim();
    if install.is_empty()
        || path.is_empty()
        || install.contains(['\0', '\n', '\r'])
        || path.contains(['\0', '\n', '\r'])
    {
        return Err("备份目录无效，未结束备份状态".to_string());
    }
    let script = shell::script(FINISH_BACKUP, &[install, path])?;
    checked(
        run_ssh_script_with_timeout(host, port, user, key, &script, Duration::from_secs(30))?,
        "结束远程备份状态",
    )?;
    Ok(())
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
    upload_staged(
        host,
        port,
        user,
        key,
        remote,
        "receive-dir",
        process::Input::Directory(local.to_path_buf()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "deploy-finish-backup-{label}-{}",
            crate::core::backup::unique_suffix()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::canonicalize(root).unwrap()
    }

    fn run_finish(root: &Path, backup: &str) -> Output {
        std::process::Command::new("bash")
            .arg("-c")
            .arg(FINISH_BACKUP)
            .arg("--")
            .arg(root.to_str().unwrap())
            .arg(backup)
            .output()
            .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn finish_backup_script_removes_marker_only_for_direct_updates() {
        let root = temp_dir("valid");
        let name = "update-20261118-101500-Ab3dE9xZ";
        let backup = root.join("backup").join(name);
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join(".updating"), b"").unwrap();

        let valid = run_finish(&root, backup.to_str().unwrap());
        assert!(
            valid.status.success(),
            "{}",
            String::from_utf8_lossy(&valid.stderr)
        );
        assert!(!backup.join(".updating").exists());

        fs::write(backup.join(".updating"), b"").unwrap();
        let traversal = format!("{}/../{name}", backup.display());
        let invalid = run_finish(&root, &traversal);
        assert!(!invalid.status.success());
        assert!(backup.join(".updating").exists());

        fs::remove_dir_all(&root).unwrap();
    }
}
