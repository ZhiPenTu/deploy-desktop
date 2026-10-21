use crate::commands::remote_ssh::{
    explain_ssh_failure, find_binary, run_ssh_script, shell_quote, ssh_base_command,
};
use std::fs::File;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

pub(super) fn create_remote_backup(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install_dir: &str,
    deploy_dir: &str,
) -> Result<String, String> {
    let script = format!(
        r#"set -e
INSTALL={install}
DEPLOY={deploy}
STAMP=$(date +%Y%m%d-%H%M%S)
BACKUP="$INSTALL/backup/update-$STAMP"
mkdir -p "$BACKUP"
if [ -f "$INSTALL/backend-java/server/target/{artifact}" ]; then
  mkdir -p "$BACKUP/server"
  cp -a "$INSTALL/backend-java/server/target/{artifact}" "$BACKUP/server/"
fi
if [ -d "$INSTALL/backend-java/gateway-app/target/classes" ]; then
  mkdir -p "$BACKUP/gateway"
  cp -a "$INSTALL/backend-java/gateway-app/target/classes" "$BACKUP/gateway/"
fi
if [ -d "$INSTALL/backend-java/gateway-app/target/lib" ]; then
  mkdir -p "$BACKUP/gateway"
  cp -a "$INSTALL/backend-java/gateway-app/target/lib" "$BACKUP/gateway/"
fi
if [ -d "$DEPLOY/frontend_dist" ]; then
  mkdir -p "$BACKUP/frontend"
  cp -a "$DEPLOY/frontend_dist" "$BACKUP/frontend/"
fi
if [ -f "$DEPLOY/docker-compose.yml" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/docker-compose.yml" "$BACKUP/deploy/"
fi
if [ -f "$DEPLOY/docker-compose.lan.yml" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/docker-compose.lan.yml" "$BACKUP/deploy/"
fi
if [ -d "$DEPLOY/nginx" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/nginx" "$BACKUP/deploy/"
fi
if [ -d "$DEPLOY/oceanbase" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/oceanbase" "$BACKUP/deploy/"
fi
echo "BACKUP_DIR=$BACKUP"
"#,
        install = shell_quote(install_dir),
        deploy = shell_quote(deploy_dir),
        artifact = crate::commands::profile::deployment_profile().backend_artifact
    );
    let output = run_ssh_script(host, port, user, key, &script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "创建远程备份失败: {}",
            explain_ssh_failure(&stderr, key)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    stdout
        .lines()
        .find_map(|line| {
            line.strip_prefix("BACKUP_DIR=")
                .map(|value| value.trim().to_string())
        })
        .ok_or_else(|| "备份完成但未返回备份目录".to_string())
}

fn upload_file(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote: &str,
) -> Result<(), String> {
    let parent = Path::new(remote)
        .parent()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());
    let mkdir_script = format!("mkdir -p {}", shell_quote(&parent));
    let mkdir_output = run_ssh_script(host, port, user, key, &mkdir_script)?;
    if !mkdir_output.status.success() {
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr)
            .trim()
            .to_string();
        return Err(explain_ssh_failure(&stderr, key));
    }

    let mut child = ssh_base_command(host, port, user, key)
        .arg(format!("cat > {}", shell_quote(remote)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动上传命令失败: {e}"))?;

    let mut file =
        File::open(local).map_err(|e| format!("打开本地文件 {} 失败: {e}", local.display()))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "无法打开 SSH 标准输入".to_string())?;
        io::copy(&mut file, stdin).map_err(|e| format!("写入远程文件失败: {e}"))?;
    }
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待上传完成失败: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!(
            "上传 {} 到 {} 失败: {}",
            local.display(),
            remote,
            explain_ssh_failure(&stderr, key)
        ))
    }
}

pub(super) fn upload_file_replace(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote: &str,
) -> Result<(), String> {
    let temp = format!("{remote}.tt-new");
    upload_file(host, port, user, key, local, &temp)?;
    let swap = format!(
        "mv -f {temp} {dest}",
        temp = shell_quote(&temp),
        dest = shell_quote(remote)
    );
    let output = run_ssh_script(host, port, user, key, &swap)?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let _ = run_ssh_script(
            host,
            port,
            user,
            key,
            &format!("rm -f {}", shell_quote(&temp)),
        );
        Err(format!(
            "替换 {} 失败: {}",
            remote,
            explain_ssh_failure(&stderr, key)
        ))
    }
}

pub(super) fn upload_dir_replace(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote: &str,
) -> Result<(), String> {
    let temp = format!("{remote}.tt-new");
    let prep = format!(
        "rm -rf {temp} && mkdir -p {temp}",
        temp = shell_quote(&temp)
    );
    let prep_output = run_ssh_script(host, port, user, key, &prep)?;
    if !prep_output.status.success() {
        let stderr = String::from_utf8_lossy(&prep_output.stderr)
            .trim()
            .to_string();
        return Err(explain_ssh_failure(&stderr, key));
    }

    let mut tar = Command::new(find_binary("tar"))
        .args(["-czf", "-", "-C"])
        .arg(local)
        .arg(".")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动本地 tar 失败: {e}"))?;
    let mut ssh = ssh_base_command(host, port, user, key)
        .arg(format!("tar -xzf - -C {}", shell_quote(&temp)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动远程解包失败: {e}"))?;

    let mut tar_stdout = tar
        .stdout
        .take()
        .ok_or_else(|| "无法读取 tar 输出".to_string())?;
    let mut ssh_stdin = ssh
        .stdin
        .take()
        .ok_or_else(|| "无法打开 SSH 标准输入".to_string())?;
    io::copy(&mut tar_stdout, &mut ssh_stdin).map_err(|e| format!("传输目录失败: {e}"))?;
    drop(ssh_stdin);

    let tar_status = tar.wait().map_err(|e| format!("等待 tar 结束失败: {e}"))?;
    let ssh_output = ssh
        .wait_with_output()
        .map_err(|e| format!("等待远程解包结束失败: {e}"))?;

    if !tar_status.success() {
        let stderr = String::from_utf8_lossy(&ssh_output.stderr)
            .trim()
            .to_string();
        let _ = run_ssh_script(
            host,
            port,
            user,
            key,
            &format!("rm -rf {}", shell_quote(&temp)),
        );
        return Err(format!(
            "打包本地目录 {} 失败{}",
            local.display(),
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        ));
    }
    if !ssh_output.status.success() {
        let stderr = String::from_utf8_lossy(&ssh_output.stderr)
            .trim()
            .to_string();
        let _ = run_ssh_script(
            host,
            port,
            user,
            key,
            &format!("rm -rf {}", shell_quote(&temp)),
        );
        return Err(format!(
            "远程解包失败: {}",
            explain_ssh_failure(&stderr, key)
        ));
    }

    let swap = format!(
        "rm -rf {dest} && mv {temp} {dest}",
        dest = shell_quote(remote),
        temp = shell_quote(&temp)
    );
    let swap_output = run_ssh_script(host, port, user, key, &swap)?;
    if swap_output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&swap_output.stderr)
            .trim()
            .to_string();
        Err(format!(
            "替换远程目录 {} 失败: {}",
            remote,
            explain_ssh_failure(&stderr, key)
        ))
    }
}
