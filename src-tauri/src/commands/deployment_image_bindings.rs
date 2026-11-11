use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::process::{self, Input, Options};

use super::bundle::sha256_hex;
use super::deployment_review::{check_local_endpoint, valid_image_reference};
use super::docker_setup::docker_command;
use super::remote_ssh::{resolve_existing_key, run_ssh_script, shell_quote};

const RECORD_FILE: &str = ".deploy-desktop/image-bindings.json";
const LIMIT: u64 = 1024 * 1024;
static BINDING_WRITE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentImageBinding {
    pub reference: String,
    pub image_id: String,
    pub platform: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BindingRecord {
    schema_version: u32,
    images: Vec<DeploymentImageBinding>,
}

pub(super) fn valid_image_id(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn normalized_platform(value: &str) -> String {
    let Some((os, architecture)) = value.trim().split_once('/') else {
        return String::new();
    };
    let architecture = match architecture {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        value => value,
    };
    format!("{os}/{architecture}")
}

pub(super) fn parse_bindings(content: &str) -> Result<Vec<DeploymentImageBinding>, String> {
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    if content.len() > LIMIT as usize {
        return Err("离线镜像记录过大，请检查部署目录后重试".to_string());
    }
    let record: BindingRecord = serde_json::from_str(content)
        .map_err(|_| "无法读取离线镜像记录，请重新传入镜像".to_string())?;
    if record.schema_version != 1 || record.images.len() > 4096 {
        return Err("离线镜像记录格式不支持，请重新传入镜像".to_string());
    }
    let mut references = HashSet::new();
    for image in &record.images {
        if !valid_image_reference(&image.reference)
            || !image.reference.contains("@sha256:")
            || !valid_image_id(&image.image_id)
            || !references.insert(image.reference.as_str())
            || !matches!(
                normalized_platform(&image.platform).as_str(),
                "linux/amd64" | "linux/arm64" | "linux/arm" | "windows/amd64"
            )
        {
            return Err("离线镜像记录无效，请重新传入对应镜像".to_string());
        }
    }
    Ok(record.images)
}

pub(super) fn local_bindings(install: &Path) -> Result<Vec<DeploymentImageBinding>, String> {
    let path = install.join(RECORD_FILE);
    for directory in [install.join(".deploy-desktop"), path.clone()] {
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("离线镜像记录不能使用符号链接，请检查部署目录".to_string())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err("无法访问离线镜像记录，请检查目录权限".to_string()),
            _ => {}
        }
    }
    let metadata = fs::metadata(&path)
        .map_err(|_| "无法访问离线镜像记录，请检查目录权限".to_string())?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err("离线镜像记录不可读取，请检查部署目录".to_string());
    }
    let content = fs::read_to_string(path)
        .map_err(|_| "无法读取离线镜像记录，请检查目录权限".to_string())?;
    parse_bindings(&content)
}

pub(crate) fn active_install_root(directory: &str) -> String {
    let profile = super::profile::deployment_profile();
    install_root_for(directory, &profile.deploy_dir, &profile.install_dir)
}

fn install_root_for(directory: &str, deploy: &str, install: &str) -> String {
    let normalize = |value: &str| value.replace('\\', "/").trim_end_matches('/').to_owned();
    if !install.trim().is_empty()
        && !deploy.trim().is_empty()
        && normalize(directory) == normalize(deploy)
    {
        install.to_owned()
    } else {
        directory.to_owned()
    }
}

pub(super) fn remote_bindings(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install: &str,
) -> Result<Vec<DeploymentImageBinding>, String> {
    let directory = format!("{}/.deploy-desktop", install.trim_end_matches('/'));
    let path = format!("{}/{}", install.trim_end_matches('/'), RECORD_FILE);
    let script = format!(
        "set -eu\n[ ! -L {directory} ] && [ ! -L {path} ] || exit 3\nif [ -e {path} ]; then\n  [ -f {path} ] && [ -r {path} ] || exit 3\n  [ \"$(wc -c < {path})\" -le {LIMIT} ] || exit 3\n  cat {path}\nfi\n",
        directory = shell_quote(&directory),
        path = shell_quote(&path),
    );
    let output = run_ssh_script(host, port, user, key, &script)?;
    if !output.status.success() {
        return Err("无法读取目标机的离线镜像记录，请检查部署目录权限".to_string());
    }
    parse_bindings(&String::from_utf8_lossy(&output.stdout))
}

pub(super) fn image_identity(reference: &str) -> Result<DeploymentImageBinding, String> {
    let mut command = docker_command();
    command.args([
        "image",
        "inspect",
        "--format",
        "{{.Id}}|{{.Os}}/{{.Architecture}}",
        "--",
        reference,
    ]);
    let output = process::run(
        command,
        Input::Bytes(Vec::new()),
        Options {
            timeout: Duration::from_secs(60),
            ..Options::default()
        },
    )?;
    if !output.status.success() {
        return Err(format!("无法读取镜像 {reference} 的内容标识，请重新拉取"));
    }
    let content = String::from_utf8_lossy(&output.stdout);
    let (image_id, platform) = content
        .trim()
        .split_once('|')
        .filter(|(image_id, _)| valid_image_id(image_id))
        .ok_or_else(|| "Docker 未返回有效的镜像内容标识，请重新检查".to_string())?;
    Ok(DeploymentImageBinding {
        reference: reference.to_string(),
        image_id: image_id.to_string(),
        platform: normalized_platform(platform),
    })
}

fn merge_bindings(
    current: Vec<DeploymentImageBinding>,
    incoming: &[DeploymentImageBinding],
) -> Result<String, String> {
    let mut merged: BTreeMap<String, DeploymentImageBinding> = current
        .into_iter()
        .map(|image| (image.reference.clone(), image))
        .collect();
    for image in incoming {
        merged.insert(image.reference.clone(), image.clone());
    }
    let content = serde_json::to_string(&BindingRecord {
        schema_version: 1,
        images: merged.into_values().collect(),
    })
    .map_err(|_| "无法保存离线镜像记录，请重试".to_string())?;
    parse_bindings(&content)?;
    Ok(content)
}

fn valid_install_dir(install: &str) -> bool {
    !install.is_empty()
        && install != "/"
        && !install.contains(['\n', '\r', '\0'])
        && crate::core::path_guard::is_absolute(install)
        && !crate::core::path_guard::has_parent_dir(Path::new(install))
}

fn write_remote_record(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install: &str,
    content: &str,
    checks: &str,
) -> Result<(), String> {
    let directory = format!("{install}/.deploy-desktop");
    let path = format!("{install}/{RECORD_FILE}");
    let script = format!(
        "set -eu\n{checks}\n[ ! -L {directory} ] && [ ! -L {path} ] || exit 3\numask 077\nmkdir -p {directory}\ntemporary=$(mktemp {temporary})\ntrap 'rm -f \"$temporary\"' EXIT\nprintf '%s' {content} > \"$temporary\"\nmv -f \"$temporary\" {path}\n",
        directory = shell_quote(&directory),
        path = shell_quote(&path),
        temporary = shell_quote(&format!("{directory}/.image-record.XXXXXXXX")),
        content = shell_quote(content)
    );
    let output = run_ssh_script(host, port, user, key, &script)?;
    if !output.status.success() {
        return Err("目标机镜像内容未通过核对或无法保存记录，请重新检查镜像与目录权限".to_string());
    }
    Ok(())
}

pub(super) fn clear_bindings(
    install: &str,
    references: &[String],
    host: Option<&str>,
    port: u16,
    user: &str,
    key_path: Option<String>,
) -> Result<(), String> {
    let install = install.trim().trim_end_matches('/');
    if !valid_install_dir(install) {
        return Err("请填写有效的目标部署目录".to_string());
    }
    let _guard = BINDING_WRITE
        .lock()
        .map_err(|_| "离线镜像记录正在更新，请稍后重试".to_string())?;
    if let Some(host) = host.filter(|host| !host.trim().is_empty()) {
        let key = resolve_existing_key(key_path, host)?;
        let current = remote_bindings(host, port, user, key.as_deref(), install)?;
        let remaining: Vec<_> = current
            .iter()
            .filter(|binding| !references.contains(&binding.reference))
            .cloned()
            .collect();
        if remaining.len() != current.len() {
            let content = merge_bindings(Vec::new(), &remaining)?;
            write_remote_record(host, port, user, key.as_deref(), install, &content, "")?;
        }
    } else {
        let root = Path::new(install);
        let current = local_bindings(root)?;
        let remaining: Vec<_> = current
            .iter()
            .filter(|binding| !references.contains(&binding.reference))
            .cloned()
            .collect();
        if remaining.len() != current.len() {
            let content = merge_bindings(Vec::new(), &remaining)?;
            write_private(&root.join(RECORD_FILE), content.as_bytes())?;
        }
    }
    Ok(())
}

fn write_private(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "离线镜像记录目录无效".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|_| "无法创建离线镜像记录目录，请检查目录权限".to_string())?;
    if fs::symlink_metadata(parent)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(true)
    {
        return Err("离线镜像记录目录不可用，请检查部署目录".to_string());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "无法准备离线镜像记录，请重试".to_string())?
        .as_nanos();
    let temporary = parent.join(format!(".image-record-{}-{stamp}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| "无法写入离线镜像记录，请检查目录权限".to_string())?;
        file.write_all(content)
            .and_then(|_| file.sync_all())
            .map_err(|_| "保存离线镜像记录失败，请检查磁盘空间".to_string())?;
        fs::rename(&temporary, path)
            .map_err(|_| "保存离线镜像记录失败，请检查目录权限".to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn remote_identity_checks(images: &[DeploymentImageBinding]) -> String {
    let mut script = String::new();
    for image in images {
        script.push_str(&format!(
            "actual=$(docker image inspect --format '{{{{.Id}}}}|{{{{.Os}}}}/{{{{.Architecture}}}}' -- {image_id} 2>/dev/null) || {{ echo '目标机缺少该离线镜像，请重新传入镜像' >&2; exit 4; }}\nactual_id=${{actual%%|*}}\nactual_platform=${{actual#*|}}\nactual_os=${{actual_platform%%/*}}\nactual_arch=${{actual_platform#*/}}\ncase \"$actual_arch\" in x86_64|amd64) actual_arch=amd64;; aarch64|arm64) actual_arch=arm64;; esac\n[ \"$actual_id\" = {expected_id} ] || {{ echo '目标机镜像内容与源端不一致，请重新传入镜像' >&2; exit 4; }}\n[ \"$actual_os/$actual_arch\" = {expected_platform} ] || {{ echo '目标机镜像架构与源端不一致，请重新传入镜像' >&2; exit 4; }}\n",
            image_id = shell_quote(&image.image_id),
            expected_id = shell_quote(&image.image_id),
            expected_platform = shell_quote(&image.platform)
        ));
    }
    script
}

#[tauri::command]
pub async fn register_deployment_image_bindings(
    bundle_dir: String,
    install_dir: String,
    references: Vec<String>,
    expected: Option<Vec<DeploymentImageBinding>>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    key_path: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = BINDING_WRITE
            .lock()
            .map_err(|_| "离线镜像记录正在更新，请稍后重试".to_string())?;
        check_local_endpoint()?;
        let declared = super::deployment_images::declared_images(&bundle_dir)?;
        let mut seen = HashSet::new();
        let mut bindings = Vec::new();
        let expected = expected.unwrap_or_default();
        let remote = host.as_deref().filter(|host| !host.trim().is_empty());
        let key = match remote {
            Some(host) => {
                let user = user
                    .as_deref()
                    .filter(|user| !user.trim().is_empty())
                    .ok_or_else(|| "请选择目标主机的 SSH 登录账号".to_string())?;
                Some((host, user, resolve_existing_key(key_path, host)?))
            }
            None => None,
        };
        for reference in references {
            let reference = reference.trim().to_owned();
            if reference.is_empty() || !seen.insert(reference.clone()) {
                continue;
            }
            if !declared.contains(&reference) {
                return Err("离线镜像未在交付包中声明，请重新核对镜像".to_string());
            }
            let binding = expected
                .iter()
                .find(|binding| binding.reference == reference)
                .ok_or_else(|| "缺少源端摘要镜像的核对记录，请重新在本机拉取并传入".to_string())?;
            let source = image_identity(&reference)?;
            if source != *binding {
                return Err("源端镜像在导出后发生变化，请重新拉取并传入".to_string());
            }
            bindings.push(source);
        }
        if bindings.is_empty() {
            return Ok(());
        }
        let validated = merge_bindings(Vec::new(), &bindings)?;
        bindings = parse_bindings(&validated)?;
        let install = install_dir.trim().trim_end_matches('/');
        if !valid_install_dir(install) {
            return Err("请填写有效的目标部署目录".to_string());
        }
        if let Some((host, user, key)) = &key {
            let host = (*host).to_string();
            let user = (*user).to_string();
            let current =
                remote_bindings(&host, port.unwrap_or(22), &user, key.as_deref(), install)?;
            let content = merge_bindings(current, &bindings)?;
            write_remote_record(
                &host,
                port.unwrap_or(22),
                &user,
                key.as_deref(),
                install,
                &content,
                &remote_identity_checks(&bindings),
            )?;
        } else {
            let root = Path::new(install);
            for image in &bindings {
                let actual = image_identity(&image.image_id)?;
                if actual.image_id != image.image_id || actual.platform != image.platform {
                    return Err("导入后的镜像内容不一致，请重新导入".to_string());
                }
            }
            let content = merge_bindings(local_bindings(root)?, &bindings)?;
            write_private(&root.join(RECORD_FILE), content.as_bytes())?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "离线镜像内容核对未完成，请重试".to_string())?
}

fn override_document(
    compose: &str,
    bindings: &[DeploymentImageBinding],
) -> Result<(String, Vec<DeploymentImageBinding>), String> {
    let services = crate::core::compose::parse_services(compose)
        .map_err(|_| "无法读取部署清单，请检查制品后重试".to_string())?;
    let mut overrides = BTreeMap::new();
    let mut used = Vec::new();
    for service in services {
        if let Some(binding) = bindings.iter().find(|image| image.reference == service.image) {
            overrides.insert(
                service.name,
                serde_json::json!({ "image": binding.image_id, "pull_policy": "never" }),
            );
            if !used.contains(binding) {
                used.push(binding.clone());
            }
        }
    }
    if used.is_empty() {
        return Ok((String::new(), used));
    }
    let document = serde_json::to_string(&serde_json::json!({ "services": overrides }))
        .map_err(|_| "无法准备离线镜像部署配置，请重试".to_string())?;
    Ok((document, used))
}

pub(crate) fn local_override_document(
    install: &Path,
    compose: &Path,
    verify_images: bool,
) -> Result<Option<String>, String> {
    let bindings = local_bindings(install)?;
    if bindings.is_empty() {
        return Ok(None);
    }
    let content = fs::read_to_string(compose)
        .map_err(|_| "无法读取部署清单，请检查目录权限".to_string())?;
    let (document, used) = override_document(&content, &bindings)?;
    if used.is_empty() {
        return Ok(None);
    }
    if verify_images {
        for expected in used {
            let actual = image_identity(&expected.image_id)?;
            if actual.image_id != expected.image_id || actual.platform != expected.platform {
                return Err("离线镜像内容或架构已变化，请重新传入镜像".to_string());
            }
        }
    }
    Ok(Some(document))
}

pub(crate) fn local_override(install: &Path, compose: &Path) -> Result<Option<PathBuf>, String> {
    let Some(document) = local_override_document(install, compose, true)? else {
        return Ok(None);
    };
    let path = install
        .join(".deploy-desktop")
        .join(format!("images-{}.compose.json", sha256_hex(document.as_bytes())));
    write_private(&path, document.as_bytes())?;
    Ok(Some(crate::core::path_guard::docker_path(&path)))
}

pub(crate) fn remote_override(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    install: &str,
    compose: &str,
) -> Result<Option<String>, String> {
    let bindings = remote_bindings(host, port, user, key, install)?;
    if bindings.is_empty() {
        return Ok(None);
    }
    let output = run_ssh_script(
        host,
        port,
        user,
        key,
        &format!(
            "set -eu\n[ -f {compose} ] && [ -r {compose} ] || exit 3\n[ \"$(wc -c < {compose})\" -le {LIMIT} ] || exit 3\ncat {compose}\n",
            compose = shell_quote(compose)
        ),
    )?;
    if !output.status.success() {
        return Err("无法读取目标机部署清单，请检查目录权限".to_string());
    }
    let (document, used) =
        override_document(&String::from_utf8_lossy(&output.stdout), &bindings)?;
    if used.is_empty() {
        return Ok(None);
    }
    let directory = format!("{}/.deploy-desktop", install.trim_end_matches('/'));
    let path = format!(
        "{directory}/images-{}.compose.json",
        sha256_hex(document.as_bytes())
    );
    let script = format!(
        "set -eu\n{checks}\n[ ! -L {directory} ] && [ ! -L {path} ] || exit 3\numask 077\nmkdir -p {directory}\ntemporary=$(mktemp {temporary})\ntrap 'rm -f \"$temporary\"' EXIT\nprintf '%s' {document} > \"$temporary\"\nmv -f \"$temporary\" {path}\n",
        checks = remote_identity_checks(&used),
        directory = shell_quote(&directory),
        path = shell_quote(&path),
        temporary = shell_quote(&format!("{directory}/.image-override.XXXXXXXX")),
        document = shell_quote(&document)
    );
    let output = run_ssh_script(host, port, user, key, &script)?;
    if !output.status.success() {
        return Err("离线镜像内容已变化或无法准备部署配置，请重新传入镜像".to_string());
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(reference: &str) -> DeploymentImageBinding {
        DeploymentImageBinding {
            reference: reference.to_string(),
            image_id: format!("sha256:{}", "b".repeat(64)),
            platform: "linux/amd64".to_string(),
        }
    }

    #[test]
    fn bindings_require_full_source_digest_and_immutable_content_id() {
        let image = binding(&format!("registry.example.com/demo/api@sha256:{}", "a".repeat(64)));
        let content = merge_bindings(Vec::new(), std::slice::from_ref(&image)).unwrap();
        assert_eq!(parse_bindings(&content).unwrap(), [image]);
        assert!(merge_bindings(Vec::new(), &[binding("demo/api:latest")]).is_err());
        let mut invalid = binding(&format!("demo/api@sha256:{}", "a".repeat(64)));
        invalid.image_id = "demo/api:latest".to_string();
        assert!(merge_bindings(Vec::new(), &[invalid]).is_err());
    }

    #[test]
    fn nested_compose_directory_uses_active_project_install_root() {
        assert_eq!(
            install_root_for("/srv/app/deploy", "/srv/app/deploy", "/srv/app"),
            "/srv/app"
        );
        assert_eq!(
            install_root_for("/srv/other/deploy", "/srv/app/deploy", "/srv/app"),
            "/srv/other/deploy"
        );
        assert_eq!(install_root_for("/srv/app", "", ""), "/srv/app");
    }

    #[test]
    fn overrides_only_apply_to_the_exact_current_source_reference() {
        let reference = format!("demo/api@sha256:{}", "a".repeat(64));
        let image = binding(&reference);
        let compose = format!("services:\n  api:\n    image: '{reference}'\n  worker:\n    image: 'demo/worker:2'\n");
        let (document, used) = override_document(&compose, std::slice::from_ref(&image)).unwrap();
        let document: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(document["services"]["api"]["image"], image.image_id);
        assert_eq!(document["services"]["api"]["pull_policy"], "never");
        assert!(document["services"].get("worker").is_none());
        assert_eq!(used.len(), 1);
        let changed = compose.replace(&"a".repeat(64), &"c".repeat(64));
        assert!(override_document(&changed, &[image]).unwrap().0.is_empty());
    }

    #[test]
    fn remote_identity_checks_quote_data_and_reject_content_changes() {
        let image = binding(&format!("demo/api@sha256:{}", "a".repeat(64)));
        let script = remote_identity_checks(&[image.clone()]);
        assert!(script.contains(&shell_quote(&image.image_id)));
        assert!(script.contains(&shell_quote("linux/amd64")));
        assert!(script.contains("actual_id"));
        assert!(script.contains("exit 4"));
    }

    #[test]
    fn direct_pull_removes_only_the_selected_offline_bindings() {
        let root = std::env::temp_dir().join(format!(
            "deploy-image-bindings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let first = binding(&format!("demo/api@sha256:{}", "a".repeat(64)));
        let second = binding(&format!("demo/worker@sha256:{}", "c".repeat(64)));
        let content = merge_bindings(Vec::new(), &[first.clone(), second.clone()]).unwrap();
        write_private(&root.join(RECORD_FILE), content.as_bytes()).unwrap();
        clear_bindings(
            root.to_str().unwrap(),
            &[first.reference],
            None,
            22,
            "",
            None,
        ).unwrap();
        assert_eq!(local_bindings(&root).unwrap(), [second]);
        fs::remove_dir_all(root).unwrap();
    }
}
