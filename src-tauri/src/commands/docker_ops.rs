use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;
use tauri::Emitter;

use super::remote_ssh::find_binary;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageLoadProgress {
    pub current: usize,
    pub total: usize,
    pub image_file: String,
    pub message: String,
    pub finished: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerStatusItem {
    pub name: String,
    pub service_key: String,
    pub state: String,       // "running", "exited", "not_created", etc.
    pub health: String,      // "healthy", "unhealthy", "starting", "none"
    pub status_text: String, // "Up 2 hours (healthy)"
    pub ports: String,
    pub image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeActionResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

const COMPOSE_CANDIDATES: [&str; 4] = [
    "docker-compose.lan.yml",
    "docker-compose.yml",
    "deploy/docker-compose.lan.yml",
    "deploy/docker-compose.yml",
];

/// 当前项目交付包声明的 Compose 清单，返回相对部署目录的路径。
///
/// 部署与监控必须使用同一份清单：交付包声明了变体（如局域网覆盖文件）时，
/// 变体才是目标环境实际运行的文件。
pub(crate) fn active_compose_relative() -> Option<String> {
    let bundle = crate::commands::projects::active_bundle()?;
    let spec = bundle.artifact_for_source(bundle.effective_compose())?;
    let path = spec.path.trim().trim_start_matches("./").to_string();
    if spec.root == crate::commands::bundle::ArtifactRoot::Deploy {
        return Some(path);
    }
    let deploy_prefix = bundle.deploy_source_rel().trim_matches('/').to_string();
    if deploy_prefix.is_empty() {
        Some(path)
    } else {
        path.strip_prefix(&format!("{deploy_prefix}/"))
            .map(str::to_string)
            .or(Some(path))
    }
}

pub fn resolve_compose_file(root: &Path) -> Option<std::path::PathBuf> {
    if let Some(relative) = active_compose_relative() {
        let declared = root.join(&relative);
        if declared.is_file() {
            return Some(declared);
        }
        let nested = root.join("deploy").join(&relative);
        if nested.is_file() {
            return Some(nested);
        }
    }
    COMPOSE_CANDIDATES
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
}

/// Keep SSH probes and local execution on the same Compose search order.
/// INSTALL is shell-quoted by the caller; all candidate names are fixed here.
pub(crate) fn compose_location_script() -> String {
    let mut script = String::from("COMPOSE_FILE=\"\"\nDEPLOY=\"$INSTALL/deploy\"\n");
    for (index, name) in COMPOSE_CANDIDATES.iter().enumerate() {
        let branch = if index == 0 { "if" } else { "elif" };
        script.push_str(&format!(
            "{branch} [ -f \"$INSTALL/{name}\" ]; then\n  COMPOSE_FILE=\"$INSTALL/{name}\"\n"
        ));
    }
    script.push_str("fi\nif [ -n \"$COMPOSE_FILE\" ]; then DEPLOY=$(dirname \"$COMPOSE_FILE\"); fi\n");
    script
}

pub fn resolve_env_files(root: &Path) -> Vec<std::path::PathBuf> {
    [
        ".env",
        "deploy/.env",
        ".env.local",
        "deploy/.env.local",
    ]
    .iter()
    .map(|name| root.join(name))
    .filter(|path| path.is_file())
    .collect()
}

/// 交付包的部署根目录不一定是 `deploy`，这里按实际部署目录补齐环境变量文件。
pub fn resolve_env_files_for(install_dir: &Path, deploy_dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = resolve_env_files(install_dir);
    for name in [".env", ".env.local"] {
        let path = deploy_dir.join(name);
        if path.is_file() && !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

/// 本机 Compose 操作可能从安装根目录或部署目录发起；环境变量文件要按
/// 「安装根目录 + 部署目录」一起解析，否则 Compose 插值会因为缺少必填变量直接失败。
pub fn resolve_env_files_for_dir(dir: &Path) -> Vec<std::path::PathBuf> {
    // 部署目录取自 Compose 清单实际所在位置，调用方传安装根目录或部署目录都成立。
    let deploy_dir = resolve_compose_file(dir)
        .and_then(|file| file.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| dir.to_path_buf());
    let install_dir = match deploy_dir.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => deploy_dir.clone(),
    };
    let mut files = resolve_env_files_for(&install_dir, &deploy_dir);
    if files.is_empty() {
        files = resolve_env_files(dir);
    }
    files
}

/// Compose 的原始输出不直接抛给用户，这里给出可操作的中文提示。
fn compose_failure_hint(stderr: &str) -> String {
    let first = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if first.contains("is missing a value") || first.contains("is not set") {
        return "Compose 配置缺少必填环境变量，请在部署目录的 .env 或 .env.local 中补全后重试"
            .to_string();
    }
    if first.contains("no such service") {
        return "Compose 清单中没有该服务，请确认所选服务与部署清单一致".to_string();
    }
    if first.contains("permission denied") {
        return "Docker 权限不足，请确认已允许当前用户访问 Docker".to_string();
    }
    "命令未执行成功，请检查部署目录与服务配置后重试".to_string()
}

/// 交付包通过 bind mount 替换产物，compose 不会因此重建容器；
/// 这里返回需要重启才能加载新产物的容器名。
pub fn artifact_containers() -> Vec<String> {
    let profile = super::profile::deployment_profile();
    profile
        .restart_services
        .iter()
        .map(|service| profile.container_name(service))
        .collect()
}

#[tauri::command]
pub async fn load_offline_images(
    window: tauri::Window,
    bundle_dir: String,
) -> Result<Vec<String>, String> {
    let images_dir = Path::new(&bundle_dir).join("images");
    if !images_dir.exists() {
        return Err(format!("镜像目录不存在: {}", images_dir.display()));
    }

    let mut tar_files = Vec::new();
    if let Ok(entries) = fs::read_dir(&images_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(ext) = p.extension() {
                if ext == "tar" || ext == "tgz" || p.to_string_lossy().ends_with(".tar.gz") {
                    tar_files.push(p);
                }
            }
        }
    }

    if tar_files.is_empty() {
        return Err("镜像目录中未找到任何 .tar 镜像文件".to_string());
    }

    tar_files.sort();
    let total = tar_files.len();
    let mut loaded = Vec::new();

    for (idx, tar_path) in tar_files.iter().enumerate() {
        let file_name = tar_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        let _ = window.emit(
            "image-load-progress",
            ImageLoadProgress {
                current: idx + 1,
                total,
                image_file: file_name.clone(),
                message: format!("正在导入镜像 [{}/{}] {} ...", idx + 1, total, file_name),
                finished: false,
            },
        );

        let output = Command::new(find_binary("docker"))
            .args(["load", "-i", &tar_path.to_string_lossy()])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
                loaded.push(format!("{}: {}", file_name, msg));
            }
            Ok(out) => {
                let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(format!("导入镜像 {} 失败: {}", file_name, err_msg));
            }
            Err(e) => {
                return Err(format!("调用 docker load 命令出错: {}", e));
            }
        }
    }

    let _ = window.emit(
        "image-load-progress",
        ImageLoadProgress {
            current: total,
            total,
            image_file: "ALL".to_string(),
            message: "全部离线 Docker 镜像加载完成！".to_string(),
            finished: true,
        },
    );

    Ok(loaded)
}

#[tauri::command]
pub async fn get_containers_status() -> Result<Vec<ContainerStatusItem>, String> {
    crate::commands::blocking::run(move || get_containers_status_blocking()).await
}

pub(crate) fn get_containers_status_blocking() -> Result<Vec<ContainerStatusItem>, String> {
    let target_containers = super::profile::deployment_profile().container_entries();

    let mut results = Vec::new();

    for (c_name, svc_key) in target_containers {
        // 使用 docker inspect 获取详细信息
        let output = Command::new(find_binary("docker"))
            .args([
                "inspect",
                "--format",
                "{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}|{{.Config.Image}}",
                &c_name,
            ])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let parts: Vec<&str> = text.split('|').collect();
                let state = parts.first().unwrap_or(&"unknown").to_string();
                let health = parts.get(1).unwrap_or(&"none").to_string();
                let image = parts.get(2).unwrap_or(&"").to_string();

                // 获取运行端口及简要状态字符串
                let ps_out = Command::new(find_binary("docker"))
                    .args(["ps", "-a", "--filter", &format!("name=^{}$", c_name), "--format", "{{.Status}}\t{{.Ports}}"])
                    .output();

                let (status_text, ports) = if let Ok(po) = ps_out {
                    let po_text = String::from_utf8_lossy(&po.stdout).trim().to_string();
                    let ps_parts: Vec<&str> = po_text.split('\t').collect();
                    (
                        ps_parts.first().unwrap_or(&"").to_string(),
                        ps_parts.get(1).unwrap_or(&"").to_string(),
                    )
                } else {
                    ("".to_string(), "".to_string())
                };

                results.push(ContainerStatusItem {
                    name: c_name,
                    service_key: svc_key,
                    state,
                    health,
                    status_text,
                    ports,
                    image,
                });
            }
            _ => {
                results.push(ContainerStatusItem {
                    name: c_name,
                    service_key: svc_key,
                    state: "not_created".to_string(),
                    health: "none".to_string(),
                    status_text: "容器未创建".to_string(),
                    ports: "".to_string(),
                    image: "".to_string(),
                });
            }
        }
    }

    Ok(results)
}

#[tauri::command]
pub fn docker_compose_action(
    deploy_dir: String,
    action: String, // "up", "stop", "restart", "down"
    service: Option<String>,
) -> Result<ComposeActionResult, String> {
    let root = Path::new(&deploy_dir);
    let compose_file = resolve_compose_file(root).ok_or_else(|| {
        format!(
            "{deploy_dir} 及其 deploy/ 子目录下均未找到 docker-compose.lan.yml 或 docker-compose.yml"
        )
    })?;
    log::info!(
        target: "compose",
        "执行 Compose 操作：动作={} 目录={} 清单={} 服务={}",
        action,
        root.display(),
        compose_file.display(),
        service.clone().unwrap_or_else(|| "全部".to_string())
    );

    let mut args = vec!["compose".to_string()];
    for env_file in resolve_env_files_for_dir(root) {
        args.push("--env-file".to_string());
        args.push(env_file.to_string_lossy().to_string());
    }
    args.push("-f".to_string());
    args.push(compose_file.to_string_lossy().to_string());

    match action.as_str() {
        "up" => {
            args.push("up".to_string());
            args.push("-d".to_string());
            // 交付包里已删除的服务需要一并回收，否则旧容器会继续占用端口与资源。
            if service.is_none() {
                args.push("--remove-orphans".to_string());
            }
            if let Some(s) = service {
                args.push(s);
            }
        }
        "stop" => {
            args.push("stop".to_string());
            if let Some(s) = service {
                args.push(s);
            }
        }
        "restart" => {
            args.push("restart".to_string());
            if let Some(s) = service {
                args.push(s);
            }
        }
        "down" => {
            args.push("down".to_string());
        }
        _ => return Err(format!("不支持的 Compose 操作: {}", action)),
    }

    let output = Command::new(find_binary("docker"))
        .args(&args)
        .current_dir(compose_file.parent().unwrap_or(root))
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let mut combined = format!("{}\n{}", stdout, stderr).trim().to_string();
            if out.status.success() {
                if action == "up" {
                    let containers = artifact_containers();
                    if !containers.is_empty() {
                        match Command::new(find_binary("docker"))
                            .arg("restart")
                            .args(&containers)
                            .output()
                        {
                            Ok(restart) if restart.status.success() => {
                                combined.push_str(&format!(
                                    "\n已重启交付产物容器: {}",
                                    containers.join(", ")
                                ));
                            }
                            Ok(restart) => {
                                let detail =
                                    String::from_utf8_lossy(&restart.stderr).trim().to_string();
                                return Ok(ComposeActionResult {
                                    success: false,
                                    output: combined,
                                    error: Some(format!("重启交付产物容器失败: {detail}")),
                                });
                            }
                            Err(e) => {
                                return Ok(ComposeActionResult {
                                    success: false,
                                    output: combined,
                                    error: Some(format!("执行 docker restart 失败: {e}")),
                                });
                            }
                        }
                    }
                }
                Ok(ComposeActionResult {
                    success: true,
                    output: combined,
                    error: None,
                })
            } else {
                log::warn!(
                    target: "compose",
                    "Compose 操作未成功：动作={} 目录={} 输出={}",
                    action,
                    deploy_dir,
                    combined.replace('\n', " ")
                );
                Ok(ComposeActionResult {
                    success: false,
                    output: combined,
                    error: Some(compose_failure_hint(&stderr)),
                })
            }
        }
        Err(e) => {
            log::error!(target: "compose", "执行 Compose 操作失败：动作={} 目录={}：{}", action, deploy_dir, e);
            Err(format!("执行 docker compose 失败: {}", e))
        }
    }
}

#[tauri::command]
pub async fn get_service_logs(container_name: String, tail: u32) -> Result<String, String> {
    crate::commands::blocking::run(move || get_service_logs_blocking(container_name, tail)).await
}

pub(crate) fn get_service_logs_blocking(container_name: String, tail: u32) -> Result<String, String> {
    let output = Command::new(find_binary("docker"))
        .args(["logs", "--tail", &tail.to_string(), &container_name])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            Ok(format!("{}\n{}", stdout, stderr).trim().to_string())
        }
        Err(e) => Err(format!("读取容器日志失败: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("deploy-desktop-{name}-{stamp}"))
    }

    #[test]
    fn resolves_compose_file_under_deploy_subdir() {
        let root = temp_root("resolve");
        fs::create_dir_all(root.join("deploy")).unwrap();
        fs::write(root.join("deploy/docker-compose.yml"), "name: lan\n").unwrap();

        let found = resolve_compose_file(&root).expect("compose file");
        assert_eq!(found, root.join("deploy/docker-compose.yml"));

        fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn ssh_and_local_compose_locations_match_for_root_and_nested_layouts() {
        let root = temp_root("compose-layout's directory");
        fs::create_dir_all(root.join("deploy")).unwrap();
        for name in [
            "deploy/docker-compose.yml", "deploy/docker-compose.lan.yml",
            "docker-compose.yml", "docker-compose.lan.yml",
        ] {
            fs::write(root.join(name), "services: {}\n").unwrap();
            let script = format!(
                "set -eu\nINSTALL={}\n{}\nprintf '%s\\n%s\\n' \"$COMPOSE_FILE\" \"$DEPLOY\"\n",
                super::super::remote_ssh::shell_quote(&root.to_string_lossy()),
                compose_location_script(),
            );
            let output = Command::new("sh").args(["-c", &script]).output().unwrap();
            assert!(output.status.success());
            let text = String::from_utf8(output.stdout).unwrap();
            let lines: Vec<_> = text.lines().collect();
            let expected = root.join(name);
            assert_eq!(resolve_compose_file(&root).unwrap(), expected);
            assert_eq!(lines[0], expected.to_str().unwrap());
            assert_eq!(lines[1], expected.parent().unwrap().to_str().unwrap());
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn compose_env_files_cover_install_root_and_deploy_dir() {
        let root = temp_root("env-files");
        fs::create_dir_all(root.join("deploy")).unwrap();
        fs::write(root.join("deploy/docker-compose.lan.yml"), "services: {}\n").unwrap();
        fs::write(root.join("deploy/.env"), "TUNNEL_TOKEN=\n").unwrap();
        fs::write(root.join(".env.local"), "OB_TENANT_PASSWORD=x\n").unwrap();

        let from_deploy = resolve_env_files_for_dir(&root.join("deploy"));
        assert_eq!(
            from_deploy,
            vec![root.join("deploy/.env"), root.join(".env.local")],
            "从部署目录发起时也要带上安装根目录的环境变量文件"
        );

        let from_root = resolve_env_files_for_dir(&root);
        assert_eq!(from_root, from_deploy);

        fs::remove_dir_all(&root).ok();
    }

}
