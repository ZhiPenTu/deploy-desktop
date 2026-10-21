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

pub fn resolve_compose_file(root: &Path) -> Option<std::path::PathBuf> {
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

const FRONTEND_OVERRIDE: &str = r#"services:
  frontend:
    image: nginx:1.27-alpine
    volumes:
      - ./frontend_dist:/usr/share/nginx/html:ro
      - ./nginx/frontend-main.conf:/etc/nginx/nginx.conf:ro
      - ./nginx/frontend-upstream.conf:/etc/nginx/conf.d/00-upstream.conf:ro
      - ./nginx/frontend-default.conf:/etc/nginx/conf.d/default.conf:ro
"#;

/// 文件模式交付包把前端产物写入 deploy/frontend_dist。若 compose 仍使用
/// 预构建镜像而不挂载该目录，交付的新前端不会生效；此时生成一个覆盖文件。
pub fn ensure_frontend_override(compose_file: &Path) -> Option<std::path::PathBuf> {
    let compose_dir = compose_file.parent()?;
    if !compose_dir.join("frontend_dist").is_dir()
        || !compose_dir.join("nginx/frontend-main.conf").is_file()
    {
        return None;
    }
    let text = fs::read_to_string(compose_file).ok()?;
    if text.contains("frontend_dist") {
        return None;
    }
    let override_path = compose_dir.join("docker-compose.frontend-bind.yml");
    fs::write(&override_path, FRONTEND_OVERRIDE).ok()?;
    Some(override_path)
}

/// 文件模式交付包已有 frontend_dist，但 compose 未挂载且缺少 nginx 配置时，
/// 直接报错，避免静默使用旧的前端镜像。
pub fn frontend_override_issue(compose_file: &Path) -> Option<String> {
    let compose_dir = compose_file.parent()?;
    if !compose_dir.join("frontend_dist").is_dir() {
        return None;
    }
    let text = fs::read_to_string(compose_file).ok()?;
    if text.contains("frontend_dist") {
        return None;
    }
    let missing: Vec<&str> = [
        "frontend-main.conf",
        "frontend-upstream.conf",
        "frontend-default.conf",
    ]
    .iter()
    .copied()
    .filter(|name| !compose_dir.join("nginx").join(name).is_file())
    .collect();
    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "交付包缺少 deploy/nginx/{}，文件模式前端无法挂载；请补齐交付包中的 nginx 配置后再启动。",
            missing.join("、")
        ))
    }
}

/// 交付包通过 bind mount 替换 jar / 网关 classes / 前端产物，compose 不会因此
/// 重建容器；这里返回需要重启才能加载新产物的容器名。
pub fn artifact_containers() -> Vec<String> {
    let profile = super::profile::deployment_profile();
    profile
        .components
        .backend
        .iter()
        .chain(profile.components.gateway.iter())
        .map(|service| format!("{}-{}", profile.container_prefix, service))
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
    if let Some(issue) = frontend_override_issue(&compose_file) {
        return Err(issue);
    }

    let mut args = vec!["compose".to_string()];
    for env_file in resolve_env_files(root) {
        args.push("--env-file".to_string());
        args.push(env_file.to_string_lossy().to_string());
    }
    args.push("-f".to_string());
    args.push(compose_file.to_string_lossy().to_string());
    if let Some(override_path) = ensure_frontend_override(&compose_file) {
        args.push("-f".to_string());
        args.push(override_path.to_string_lossy().to_string());
    }

    match action.as_str() {
        "up" => {
            args.push("up".to_string());
            args.push("-d".to_string());
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
                Ok(ComposeActionResult {
                    success: false,
                    output: combined,
                    error: Some("命令返回非零退出码".to_string()),
                })
            }
        }
        Err(e) => Err(format!("执行 docker compose 失败: {}", e)),
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
    fn writes_frontend_override_for_file_mode_bundle() {
        let root = temp_root("override");
        let deploy = root.join("deploy");
        fs::create_dir_all(deploy.join("nginx")).unwrap();
        fs::create_dir_all(deploy.join("frontend_dist")).unwrap();
        fs::write(deploy.join("nginx/frontend-main.conf"), "events {}\n").unwrap();
        let compose = deploy.join("docker-compose.lan.yml");
        fs::write(
            &compose,
            "services:\n  frontend:\n    image: app-frontend:dev\n",
        )
        .unwrap();

        let override_path = ensure_frontend_override(&compose).expect("override");
        let text = fs::read_to_string(&override_path).unwrap();
        assert!(text.contains("nginx:1.27-alpine"));
        assert!(text.contains("frontend_dist:/usr/share/nginx/html"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn skips_override_when_compose_already_mounts_frontend_dist() {
        let root = temp_root("no-override");
        let deploy = root.join("deploy");
        fs::create_dir_all(deploy.join("nginx")).unwrap();
        fs::create_dir_all(deploy.join("frontend_dist")).unwrap();
        fs::write(deploy.join("nginx/frontend-main.conf"), "events {}\n").unwrap();
        let compose = deploy.join("docker-compose.yml");
        fs::write(&compose, "services:\n  frontend:\n    volumes:\n      - ./frontend_dist:/usr/share/nginx/html:ro\n")
            .unwrap();

        assert!(ensure_frontend_override(&compose).is_none());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reports_missing_nginx_config_instead_of_silent_fallback() {
        let root = temp_root("missing-conf");
        let deploy = root.join("deploy");
        fs::create_dir_all(deploy.join("nginx")).unwrap();
        fs::create_dir_all(deploy.join("frontend_dist")).unwrap();
        let compose = deploy.join("docker-compose.lan.yml");
        fs::write(&compose, "services:\n  frontend:\n    image: app-frontend:dev\n").unwrap();

        let issue = frontend_override_issue(&compose).expect("issue");
        assert!(issue.contains("frontend-main.conf"));
        assert!(issue.contains("frontend-default.conf"));

        fs::remove_dir_all(&root).ok();
    }
}
