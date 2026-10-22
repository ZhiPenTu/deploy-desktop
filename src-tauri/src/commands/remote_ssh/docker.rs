//! 远程 Docker 引擎安装、镜像导入与 Compose 操作。

use super::*;

pub(crate) fn install_remote_docker_engine_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
) -> Result<DockerInstallResult, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let engine_dir = format!("{}/docker-engine", bundle_dir.trim_end_matches('/'));
    let mut steps = Vec::new();

    let step1 = format!(
        "set -e\nENGINE={}\n[ -d \"$ENGINE\" ] || {{ echo \"未找到离线安装目录: $ENGINE\" >&2; exit 3; }}\necho \"找到离线资源目录: $ENGINE\"\nls -1 \"$ENGINE\"\n",
        shell_quote(&engine_dir)
    );
    let step1_out = run_ssh_script(&host, port, &user, key.as_deref(), &step1)?;
    if !step1_out.status.success() {
        let message = String::from_utf8_lossy(&step1_out.stderr)
            .trim()
            .to_string();
        return Ok(DockerInstallResult {
            success: false,
            steps: vec![DockerInstallStep {
                step: "1. 扫描 Docker 离线静态二进制包".to_string(),
                status: "failed".to_string(),
                message,
            }],
            docker_version: None,
            error_message: Some("远程主机缺少 docker-engine 离线安装目录".to_string()),
        });
    }
    steps.push(DockerInstallStep {
        step: "1. 扫描 Docker 离线静态二进制包".to_string(),
        status: "success".to_string(),
        message: format!("找到离线资源目录: {engine_dir}"),
    });

    let step2 = format!(
        r##"set -e
ENGINE={}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
TGZ=$(find "$ENGINE" -maxdepth 1 -type f \( -name '*.tgz' -o -name '*.tar.gz' \) | head -1)
if [ -n "$TGZ" ]; then
  tar -zxf "$TGZ" -C "$TMP"
  SRC="$TMP/docker"
else
  SRC="$ENGINE"
fi
if [ ! -f "$SRC/docker" ] && [ ! -f "$SRC/dockerd" ]; then
  echo "未找到 docker 静态二进制文件" >&2
  exit 4
fi
cp -a "$SRC/." /usr/bin/
for binary in docker dockerd containerd runc ctr; do
  [ -f "/usr/bin/$binary" ] && chmod +x "/usr/bin/$binary"
done
if [ -f "$ENGINE/docker-compose" ]; then
  mkdir -p /usr/local/bin /usr/libexec/docker/cli-plugins
  cp -f "$ENGINE/docker-compose" /usr/local/bin/docker-compose
  chmod +x /usr/local/bin/docker-compose
  cp -f "$ENGINE/docker-compose" /usr/libexec/docker/cli-plugins/docker-compose
  chmod +x /usr/libexec/docker/cli-plugins/docker-compose
fi
echo DOCKER_BINARIES_READY
"##,
        shell_quote(&engine_dir)
    );
    let step2_out = run_ssh_script(&host, port, &user, key.as_deref(), &step2)?;
    if !step2_out.status.success() {
        let message = String::from_utf8_lossy(&step2_out.stderr)
            .trim()
            .to_string();
        steps.push(DockerInstallStep {
            step: "2. 安装 Docker 核心二进制文件".to_string(),
            status: "failed".to_string(),
            message,
        });
        return Ok(DockerInstallResult {
            success: false,
            steps,
            docker_version: None,
            error_message: Some("远程主机安装 Docker 二进制失败".to_string()),
        });
    }
    steps.push(DockerInstallStep {
        step: "2. 安装 Docker 核心二进制文件".to_string(),
        status: "success".to_string(),
        message: "Docker 与 Compose 二进制已部署至远程主机".to_string(),
    });

    let step3 = r##"set -e
mkdir -p /etc/systemd/system /etc/docker
cat > /etc/systemd/system/docker.service <<'UNIT'
[Unit]
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
UNIT
if [ ! -f /etc/docker/daemon.json ]; then
  cat > /etc/docker/daemon.json <<'JSON'
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "100m",
    "max-file": "3"
  },
  "live-restore": true
}
JSON
fi
echo SYSTEMD_CONFIG_READY
"##;
    let step3_out = run_ssh_script(&host, port, &user, key.as_deref(), step3)?;
    if !step3_out.status.success() {
        let message = String::from_utf8_lossy(&step3_out.stderr)
            .trim()
            .to_string();
        steps.push(DockerInstallStep {
            step: "3. 配置 systemd 服务与 Daemon 参数".to_string(),
            status: "failed".to_string(),
            message,
        });
        return Ok(DockerInstallResult {
            success: false,
            steps,
            docker_version: None,
            error_message: Some("远程主机写入 Docker systemd 配置失败".to_string()),
        });
    }
    steps.push(DockerInstallStep {
        step: "3. 配置 systemd 服务与 Daemon 参数".to_string(),
        status: "success".to_string(),
        message: "systemd 单元与 daemon.json 已写入".to_string(),
    });

    let step4 =
        "set -e\nsystemctl daemon-reload\nsystemctl enable --now docker\ndocker --version\n";
    let step4_out = run_ssh_script(&host, port, &user, key.as_deref(), step4)?;
    if !step4_out.status.success() {
        let message = String::from_utf8_lossy(&step4_out.stderr)
            .trim()
            .to_string();
        steps.push(DockerInstallStep {
            step: "4. 启动 Docker 服务并验证运行状态".to_string(),
            status: "failed".to_string(),
            message,
        });
        return Ok(DockerInstallResult {
            success: false,
            steps,
            docker_version: None,
            error_message: Some("远程主机启动 Docker 服务失败".to_string()),
        });
    }
    let docker_version = String::from_utf8_lossy(&step4_out.stdout)
        .trim()
        .to_string();
    steps.push(DockerInstallStep {
        step: "4. 启动 Docker 服务并验证运行状态".to_string(),
        status: "success".to_string(),
        message: format!("Docker 服务已启动: {docker_version}"),
    });

    Ok(DockerInstallResult {
        success: true,
        steps,
        docker_version: Some(docker_version),
        error_message: None,
    })
}

pub(crate) fn remote_docker_compose_action_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    deploy_dir: String,
    action: String,
    service: Option<String>,
) -> Result<ComposeActionResult, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let mut result = remote_compose_action(
        &host,
        port,
        &user,
        key.as_deref(),
        &deploy_dir,
        &action,
        service,
    )?;
    if action == "up" && result.success {
        let containers = crate::commands::docker_ops::artifact_containers();
        if !containers.is_empty() {
            let names = containers
                .iter()
                .map(|name| shell_quote(name))
                .collect::<Vec<_>>()
                .join(" ");
            let restart = run_ssh_script(
                &host,
                port,
                &user,
                key.as_deref(),
                &format!("docker restart {names}"),
            )?;
            if restart.status.success() {
                result
                    .output
                    .push_str(&format!("\n已重启交付产物容器: {}", containers.join(", ")));
            } else {
                let stderr = String::from_utf8_lossy(&restart.stderr).trim().to_string();
                result.success = false;
                result.error = Some(format!("重启交付产物容器失败: {stderr}"));
            }
        }
    }
    Ok(result)
}
