use serde::{Deserialize, Serialize};
use std::net::{TcpListener, UdpSocket};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortStatus {
    pub port: u16,
    pub protocol: String,
    pub service: String,
    pub available: bool,
    pub occupying_process: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalEnvInfo {
    pub os: String,
    pub arch: String,
    pub os_release: String,
    pub cpu_cores: usize,
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub disk_available_gb: f64,
    pub docker_installed: bool,
    pub docker_version: Option<String>,
    pub docker_running: bool,
    pub compose_installed: bool,
    pub compose_version: Option<String>,
    pub ports: Vec<PortStatus>,
    pub is_ready: bool,
    pub issues: Vec<String>,
}

#[tauri::command]
pub fn check_local_env(target_dir: Option<String>) -> Result<LocalEnvInfo, String> {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    // 获取系统发行版描述
    let os_release = get_os_release(&os);

    // 获取内存大小
    let (total_memory_mb, available_memory_mb) = get_memory_info(&os);

    // 获取磁盘剩余空间 (GB)
    let check_path = target_dir.unwrap_or_else(|| {
        if os == "linux" {
            "/home".to_string()
        } else {
            ".".to_string()
        }
    });
    let disk_available_gb = get_disk_free_gb(&check_path);

    // 检测 Docker
    let (docker_installed, docker_version, docker_running) = check_docker_status();
    let (compose_installed, compose_version) = check_docker_compose_status();

    // 检测核心端口占用情况
    let mut ports = Vec::new();
    let core_ports = super::profile::deployment_profile().port_definitions();

    for (p, proto, svc) in core_ports {
        let (avail, occupant) = check_port_availability(p, proto);
        ports.push(PortStatus {
            port: p,
            protocol: proto.to_string(),
            service: svc.to_string(),
            available: avail,
            occupying_process: occupant,
        });
    }

    let mut issues = Vec::new();
    if !docker_installed {
        issues.push("未检测到 Docker 运行环境 (需执行离线安装 Docker)".to_string());
    } else if !docker_running {
        issues.push("Docker 服务未处于运行状态 (需启动 dockerd / systemctl start docker)".to_string());
    }

    if !compose_installed {
        issues.push("未检测到 Docker Compose 插件".to_string());
    }

    if total_memory_mb > 0 && total_memory_mb < 8192 {
        issues.push(format!("宿主机物理内存为 {}MB, 建议至少 16GB 以平稳运行 OceanBase 容器", total_memory_mb));
    }

    if disk_available_gb > 0.0 && disk_available_gb < 20.0 {
        issues.push(format!("目标挂载分区磁盘剩余空间仅 {:.1}GB, 建议预留 30GB 以上", disk_available_gb));
    }

    for p in &ports {
        if !p.available {
            // 如果是被自身的容器占用，给出提示而不是致命错误
            let occ = p.occupying_process.as_deref().unwrap_or("未知进程");
            issues.push(format!("{} 端口 {} 已被占用 ({})", p.protocol, p.port, occ));
        }
    }

    let is_ready = docker_installed && docker_running && compose_installed;

    Ok(LocalEnvInfo {
        os,
        arch,
        os_release,
        cpu_cores,
        total_memory_mb,
        available_memory_mb,
        disk_available_gb,
        docker_installed,
        docker_version,
        docker_running,
        compose_installed,
        compose_version,
        ports,
        is_ready,
        issues,
    })
}

fn get_os_release(os: &str) -> String {
    if os == "linux" {
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if line.starts_with("PRETTY_NAME=") {
                    return line.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string();
                }
            }
        }
        "Linux (通用)".to_string()
    } else if os == "macos" {
        let output = Command::new("sw_vers").arg("-productVersion").output();
        if let Ok(out) = output {
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            format!("macOS {}", ver)
        } else {
            "macOS".to_string()
        }
    } else {
        os.to_string()
    }
}

fn get_memory_info(os: &str) -> (u64, u64) {
    if os == "linux" {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            let mut total = 0u64;
            let mut available = 0u64;
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        total = parts[1].parse::<u64>().unwrap_or(0) / 1024;
                    }
                } else if line.starts_with("MemAvailable:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        available = parts[1].parse::<u64>().unwrap_or(0) / 1024;
                    }
                }
            }
            return (total, available);
        }
    } else if os == "macos" {
        if let Ok(output) = Command::new("sysctl").args(["-n", "hw.memsize"]).output() {
            let str_val = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Ok(bytes) = str_val.parse::<u64>() {
                let total = bytes / 1024 / 1024;
                return (total, total / 2); // 估算
            }
        }
    }
    (0, 0)
}

fn get_disk_free_gb(path: &str) -> f64 {
    let output = Command::new("df").args(["-k", path]).output();
    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() >= 2 {
            let parts: Vec<&str> = lines[1].split_whitespace().collect();
            // 在 df -k 输出中，第4列通常是可用容量 (1K-blocks)
            if parts.len() >= 4 {
                if let Ok(avail_kb) = parts[3].parse::<f64>() {
                    return (avail_kb / 1024.0 / 1024.0 * 10.0).round() / 10.0;
                }
            }
        }
    }
    0.0
}

fn check_docker_status() -> (bool, Option<String>, bool) {
    let ver_res = Command::new("docker").arg("--version").output();
    if let Ok(out) = ver_res {
        if out.status.success() {
            let ver_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // 检测 daemon 是否运行
            let ping_res = Command::new("docker").arg("info").output();
            let is_running = ping_res.map(|o| o.status.success()).unwrap_or(false);
            return (true, Some(ver_str), is_running);
        }
    }
    (false, None, false)
}

fn check_docker_compose_status() -> (bool, Option<String>) {
    let res = Command::new("docker").args(["compose", "version"]).output();
    if let Ok(out) = res {
        if out.status.success() {
            let ver_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return (true, Some(ver_str));
        }
    }
    // 兼容独立 docker-compose 命令
    let res2 = Command::new("docker-compose").arg("--version").output();
    if let Ok(out) = res2 {
        if out.status.success() {
            let ver_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return (true, Some(ver_str));
        }
    }
    (false, None)
}

fn check_port_availability(port: u16, proto: &str) -> (bool, Option<String>) {
    if proto == "TCP" {
        match TcpListener::bind(("0.0.0.0", port)) {
            Ok(_) => (true, None),
            Err(_) => {
                // 探测被哪个进程或容器占用
                let occ = probe_port_occupant(port);
                (false, Some(occ))
            }
        }
    } else {
        match UdpSocket::bind(("0.0.0.0", port)) {
            Ok(_) => (true, None),
            Err(_) => {
                let occ = probe_port_occupant(port);
                (false, Some(occ))
            }
        }
    }
}

fn probe_port_occupant(port: u16) -> String {
    // 优先检查 docker ps 是否映射此端口
    let port_str = format!(":{}", port);
    if let Ok(out) = Command::new("docker").args(["ps", "--format", "{{.Names}}\t{{.Ports}}"]).output() {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if line.contains(&port_str) {
                let parts: Vec<&str> = line.split('\t').collect();
                if !parts.is_empty() {
                    return format!("容器 {}", parts[0]);
                }
            }
        }
    }
    // 尝试 lsof 探测
    if let Ok(out) = Command::new("lsof").args(["-i", &format!(":{}", port)]).output() {
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() >= 2 {
            let parts: Vec<&str> = lines[1].split_whitespace().collect();
            if !parts.is_empty() {
                return format!("进程 {}", parts[0]);
            }
        }
    }
    "占用中".to_string()
}
