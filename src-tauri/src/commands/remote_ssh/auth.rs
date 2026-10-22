//! 免密登录的初始化：生成或复用密钥，并把公钥写入目标主机的授权列表。

use std::fs;

use super::*;

pub(crate) fn setup_remote_ssh_key_blocking(
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    key_path: Option<String>,
) -> Result<SshTestResult, String> {
    let target_key = resolve_key_target(key_path, &host);

    let pub_key_path = format!("{}.pub", target_key);

    // 如果私钥不存在，自动生成 ED25519 密钥对
    if !Path::new(&target_key).is_file() {
        if let Some(parent) = Path::new(&target_key).parent() {
            let _ = fs::create_dir_all(parent);
        }
        let gen_res = Command::new(find_binary("ssh-keygen"))
            .args([
                "-t",
                "ed25519",
                "-N",
                "",
                "-f",
                &target_key,
                "-C",
                &format!("deploy-desktop@{}", host),
            ])
            .output();
        match gen_res {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                return Err(format!(
                    "生成 SSH 密钥对失败: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Err(e) => return Err(format!("生成 SSH 密钥对失败: {}", e)),
        }
    }

    // 私钥已经可用时直接返回成功，避免重复下发
    if let Ok(result) =
        test_ssh_connection_blocking(host.clone(), port, user.clone(), Some(target_key.clone()))
    {
        if result.success {
            return Ok(SshTestResult {
                success: true,
                host,
                user,
                message: format!(
                    "当前密钥已可免密登录，无需重复部署。本地私钥: {}",
                    target_key
                ),
                latency_ms: result.latency_ms,
            });
        }
    }

    let pub_content = fs::read_to_string(&pub_key_path)
        .map_err(|e| format!("读取公钥文件 {} 失败: {}", pub_key_path, e))?
        .trim()
        .to_string();

    let remote_sh = remote_authorized_keys_script(&pub_content);
    let mut deploy_error = None;

    if let Some(pwd) = password.filter(|p| !p.is_empty()) {
        match deploy_public_key_with_password(&host, port, &user, &pwd, &remote_sh) {
            Ok(()) => {
                let verify = test_ssh_connection_blocking(
                    host.clone(),
                    port,
                    user.clone(),
                    Some(target_key.clone()),
                )?;
                if verify.success {
                    return Ok(SshTestResult {
                        success: true,
                        host,
                        user,
                        message: format!("公钥已部署并通过免密登录验证。本地私钥: {}", target_key),
                        latency_ms: verify.latency_ms,
                    });
                }
                deploy_error = Some(verify.message);
            }
            Err(e) => deploy_error = Some(e),
        }
    }

    Ok(SshTestResult {
        success: false,
        host,
        user,
        message: match deploy_error {
            Some(e) => format!(
                "自动下发公钥失败: {}\n请手动将以下公钥追加到远程的 ~/.ssh/authorized_keys:\n{}",
                e, pub_content
            ),
            None => format!(
                "未填写初始密码，无法自动下发公钥。请手动将以下公钥追加到远程的 ~/.ssh/authorized_keys:\n{}",
                pub_content
            ),
        },
        latency_ms: None,
    })
}

pub(crate) fn resolve_existing_key(
    custom: Option<String>,
    host: &str,
) -> Result<Option<String>, String> {
    if let Some(raw) = custom {
        if !raw.trim().is_empty() {
            let path = expand_tilde(&raw);
            if !Path::new(&path).is_file() {
                return Err(format!("指定的私钥文件不存在: {path}"));
            }
            ensure_key_permissions(&path)?;
            return Ok(Some(path));
        }
    }

    let host_specific = default_key_path(host);
    if Path::new(&host_specific).is_file() {
        ensure_key_permissions(&host_specific)?;
        return Ok(Some(host_specific));
    }

    if let Some(home) = dirs::home_dir() {
        for name in ["id_ed25519", "id_rsa"] {
            let candidate = home.join(".ssh").join(name);
            if candidate.is_file() {
                let path = candidate.to_string_lossy().to_string();
                ensure_key_permissions(&path)?;
                return Ok(Some(path));
            }
        }
    }

    Ok(None)
}

pub(crate) fn remote_authorized_keys_script(public_key: &str) -> String {
    format!(
        r#"set -e
umask 077
mkdir -p ~/.ssh
chmod 700 ~/.ssh
touch ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
KEY={}
grep -qxF "$KEY" ~/.ssh/authorized_keys || printf '%s\n' "$KEY" >> ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
echo KEY_INSTALLED
"#,
        shell_quote(public_key)
    )
}

pub(crate) fn deploy_public_key_with_password(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    remote_script: &str,
) -> Result<(), String> {
    let encoded = base64_encode(remote_script)?;
    let remote_command = format!("printf %s {encoded} | base64 -d | bash");
    let sshpass = find_binary("sshpass");

    if Path::new(&sshpass).is_file() {
        let output = Command::new(sshpass)
            .env("SSHPASS", password)
            .args([
                "-e",
                "ssh",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "LogLevel=ERROR",
                "-o",
                "PreferredAuthentications=password,keyboard-interactive",
                "-o",
                "PubkeyAuthentication=no",
                "-o",
                "ConnectTimeout=8",
                "-p",
                &port.to_string(),
                &format!("{user}@{host}"),
                &remote_command,
            ])
            .output()
            .map_err(|e| format!("执行 sshpass 失败: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !stderr.is_empty() {
            return Err(stderr);
        }
    }

    let expect = find_binary("expect");
    if !Path::new(&expect).is_file() {
        return Err("系统未安装 sshpass 或 expect，无法自动使用密码登录".to_string());
    }

    let expect_script = r#"set timeout 25
set password $env(DEPLOY_SSH_PASSWORD)
set host $env(DEPLOY_SSH_HOST)
set port $env(DEPLOY_SSH_PORT)
set user $env(DEPLOY_SSH_USER)
set b64 $env(DEPLOY_SSH_SCRIPT_B64)
spawn ssh -o StrictHostKeyChecking=no -o LogLevel=ERROR -o PreferredAuthentications=password,keyboard-interactive -o PubkeyAuthentication=no -o ConnectTimeout=8 -p $port $user@$host "printf %s $b64 | base64 -d | bash"
expect {
  -re "(?i)password.*:" { send -- "$password\r"; exp_continue }
  -re "(?i)permission denied" { exit 2 }
  -re "(?i)connection refused" { exit 3 }
  timeout { exit 4 }
  eof
}
catch wait result
exit [lindex $result 3]
"#;

    let script_path =
        std::env::temp_dir().join(format!("deploy-ssh-key-{}.exp", std::process::id()));
    fs::write(&script_path, expect_script).map_err(|e| format!("写入 expect 脚本失败: {e}"))?;
    let output = Command::new(expect)
        .arg(&script_path)
        .env("DEPLOY_SSH_PASSWORD", password)
        .env("DEPLOY_SSH_HOST", host)
        .env("DEPLOY_SSH_PORT", port.to_string())
        .env("DEPLOY_SSH_USER", user)
        .env("DEPLOY_SSH_SCRIPT_B64", &encoded)
        .output();
    let _ = fs::remove_file(&script_path);

    let output = output.map_err(|e| format!("执行 expect 失败: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}
