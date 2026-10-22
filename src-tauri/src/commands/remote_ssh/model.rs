//! SSH 相关命令的返回结构；字段由界面直接消费，名称保持不变。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTestResult {
    pub success: bool,
    pub host: String,
    pub user: String,
    pub message: String,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEnvResult {
    pub success: bool,
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
    pub ports: Vec<RemotePortStatus>,
    pub is_ready: bool,
    pub raw_output: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePortStatus {
    pub port: u16,
    pub protocol: String,
    pub service: String,
    pub available: bool,
    pub occupying_process: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshDeployExecutionResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}
