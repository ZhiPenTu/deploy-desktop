use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerInstallStep {
    pub step: String,
    pub status: String, // "pending", "running", "success", "failed"
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerInstallResult {
    pub success: bool,
    pub steps: Vec<DockerInstallStep>,
    pub docker_version: Option<String>,
    pub error_message: Option<String>,
}

#[tauri::command]
pub async fn install_offline_docker_engine(
    bundle_dir: String,
) -> Result<DockerInstallResult, String> {
    crate::commands::blocking::run(move || install_offline_docker_engine_blocking(bundle_dir)).await
}

pub(crate) fn install_offline_docker_engine_blocking(
    bundle_dir: String,
) -> Result<DockerInstallResult, String> {
    let mut steps = Vec::new();
    let os = std::env::consts::OS;

    if os != "linux" {
        return Ok(DockerInstallResult {
            success: false,
            steps: vec![DockerInstallStep {
                step: "系统架构检查".to_string(),
                status: "failed".to_string(),
                message: format!("离线静默安装 Docker 引擎仅适用于 Linux x86_64 服务器，当前系统为 {}", os),
            }],
            docker_version: None,
            error_message: Some(format!("非 Linux 环境无需离线安装 systemd Docker 引擎 (当前: {})", os)),
        });
    }

    let bundle_path = Path::new(&bundle_dir);
    let engine_dir = bundle_path.join("docker-engine");

    // 步骤 1: 扫描离线安装包
    let mut step1 = DockerInstallStep {
        step: "1. 扫描 Docker 离线静态二进制包".to_string(),
        status: "running".to_string(),
        message: "正在查找 docker-*.tgz 或预解压文件...".to_string(),
    };

    if !engine_dir.exists() {
        step1.status = "failed".to_string();
        step1.message = format!("未找到离线安装目录: {}", engine_dir.display());
        steps.push(step1);
        return Ok(DockerInstallResult {
            success: false,
            steps,
            docker_version: None,
            error_message: Some("离线安装包缺失 docker-engine 目录".to_string()),
        });
    }

    // 寻找 tgz 包或已存在的文件
    let mut tarball_found = None;
    if let Ok(entries) = fs::read_dir(&engine_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(ext) = p.extension() {
                if ext == "tgz" || p.to_string_lossy().ends_with(".tar.gz") {
                    tarball_found = Some(p);
                    break;
                }
            }
        }
    }

    step1.status = "success".to_string();
    step1.message = format!("找到离线资源目录: {}", engine_dir.display());
    steps.push(step1);

    // 步骤 2: 解压二进制并复制到 /usr/bin
    let mut step2 = DockerInstallStep {
        step: "2. 安装 Docker 核心二进制文件".to_string(),
        status: "running".to_string(),
        message: "正在部署 dockerd, docker, containerd, runc 至 /usr/bin/ ...".to_string(),
    };

    if let Some(tgz) = tarball_found {
        let extract_res = Command::new("tar")
            .args(["-zxf", &tgz.to_string_lossy(), "-C", "/tmp"])
            .output();
        if let Err(e) = extract_res {
            step2.status = "failed".to_string();
            step2.message = format!("解压 Docker 二进制包失败: {}", e);
            steps.push(step2);
            return Ok(DockerInstallResult {
                success: false,
                steps,
                docker_version: None,
                error_message: Some("解压失败".to_string()),
            });
        }

        // 将 /tmp/docker/* 复制到 /usr/bin/
        let cp_res = Command::new("cp")
            .args(["-a", "/tmp/docker/.", "/usr/bin/"])
            .output();
        if let Err(e) = cp_res {
            step2.status = "failed".to_string();
            step2.message = format!("复制二进制文件至 /usr/bin/ 失败 (请确认是否有 root 权限): {}", e);
            steps.push(step2);
            return Ok(DockerInstallResult {
                success: false,
                steps,
                docker_version: None,
                error_message: Some("权限不足或复制失败".to_string()),
            });
        }
    } else {
        // 直接从 engine_dir 寻找 docker 可执行文件
        let direct_docker = engine_dir.join("docker");
        if direct_docker.exists() {
            let cp_res = Command::new("cp")
                .args(["-a", &format!("{}/.", engine_dir.display()), "/usr/bin/"])
                .output();
            if let Err(e) = cp_res {
                step2.status = "failed".to_string();
                step2.message = format!("复制文件失败: {}", e);
                steps.push(step2);
                return Ok(DockerInstallResult {
                    success: false,
                    steps,
                    docker_version: None,
                    error_message: Some("复制失败".to_string()),
                });
            }
        } else {
            step2.status = "failed".to_string();
            step2.message = "未在离线目录中找到 docker 静态安装包或可执行文件".to_string();
            steps.push(step2);
            return Ok(DockerInstallResult {
                success: false,
                steps,
                docker_version: None,
                error_message: Some("离线文件不完整".to_string()),
            });
        }
    }

    // 安装 docker-compose 单文件插件
    let compose_src = engine_dir.join("docker-compose");
    if compose_src.exists() {
        let _ = fs::create_dir_all("/usr/local/bin");
        let _ = fs::create_dir_all("/usr/libexec/docker/cli-plugins");
        let _ = Command::new("cp").args([&compose_src.to_string_lossy(), "/usr/local/bin/docker-compose"]).output();
        let _ = Command::new("chmod").args(["+x", "/usr/local/bin/docker-compose"]).output();
        let _ = Command::new("cp").args([&compose_src.to_string_lossy(), "/usr/libexec/docker/cli-plugins/docker-compose"]).output();
        let _ = Command::new("chmod").args(["+x", "/usr/libexec/docker/cli-plugins/docker-compose"]).output();
    }

    step2.status = "success".to_string();
    step2.message = "Docker 与 Compose 二进制部署完成并赋予执行权限".to_string();
    steps.push(step2);

    // 步骤 3: 注册 systemd 服务并配置守护进程
    let mut step3 = DockerInstallStep {
        step: "3. 配置 systemd 服务与 Daemon 参数".to_string(),
        status: "running".to_string(),
        message: "正在写入 /etc/systemd/system/docker.service 与 daemon.json ...".to_string(),
    };

    let docker_service_content = r#"[Unit]
Description=Docker Application Container Engine
Documentation=https://docs.docker.com
After=network-online.target firewalld.service containerd.service
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/bin/dockerd
ExecReload=/bin/kill -s HUP $MAINPID
TimeoutSec=0
RestartSec=2
Restart=always
LimitNOFILE=infinity
LimitNPROC=infinity
LimitCORE=infinity
TasksMax=infinity
Delegate=yes
KillMode=process
OOMScoreAdjust=-500

[Install]
WantedBy=multi-user.target
"#;

    let _ = fs::write("/etc/systemd/system/docker.service", docker_service_content);

    // 写入默认 daemon.json (优化日志轮转与数据存储)
    let _ = fs::create_dir_all("/etc/docker");
    let daemon_json = r#"{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "100m",
    "max-file": "3"
  },
  "live-restore": true
}
"#;
    let _ = fs::write("/etc/docker/daemon.json", daemon_json);

    step3.status = "success".to_string();
    step3.message = "Systemd 单元配置及日志策略写入完成".to_string();
    steps.push(step3);

    // 步骤 4: 重载并拉起 Docker 服务
    let mut step4 = DockerInstallStep {
        step: "4. 启动 Docker 服务并验证运行状态".to_string(),
        status: "running".to_string(),
        message: "正在执行 systemctl daemon-reload && systemctl enable --now docker ...".to_string(),
    };

    let _ = Command::new("systemctl").arg("daemon-reload").output();
    let start_res = Command::new("systemctl").args(["enable", "--now", "docker"]).output();

    if let Err(e) = start_res {
        step4.status = "failed".to_string();
        step4.message = format!("启动 docker 服务失败: {}", e);
        steps.push(step4);
        return Ok(DockerInstallResult {
            success: false,
            steps,
            docker_version: None,
            error_message: Some(format!("启动失败: {}", e)),
        });
    }

    // 等待 2 秒后探测 docker info
    std::thread::sleep(std::time::Duration::from_millis(2000));
    let info_res = Command::new("docker").arg("info").output();
    let version_res = Command::new("docker").arg("--version").output();

    let ver_str = version_res
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());

    if let Ok(info) = info_res {
        if info.status.success() {
            step4.status = "success".to_string();
            step4.message = format!("Docker 守护进程成功拉起并处于正常活跃状态 ({})", ver_str.as_deref().unwrap_or(""));
            steps.push(step4);
            return Ok(DockerInstallResult {
                success: true,
                steps,
                docker_version: ver_str,
                error_message: None,
            });
        }
    }

    step4.status = "failed".to_string();
    step4.message = "Docker 守护进程启动超时，请使用 journalctl -u docker 检查日志".to_string();
    steps.push(step4);

    Ok(DockerInstallResult {
        success: false,
        steps,
        docker_version: ver_str,
        error_message: Some("Docker 启动检测失败".to_string()),
    })
}
