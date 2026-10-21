use crate::commands::docker_ops::ContainerStatusItem;
use serde::{Deserialize, Serialize};

/// 目标机上单个产物的实际状态，字段来自交付包描述符，不再固定为 jar / classes / dist。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteArtifactStatus {
    pub key: String,
    pub label: String,
    pub root: String,
    pub path: String,
    pub exists: bool,
    pub size_bytes: u64,
    pub modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDeploymentStatus {
    pub deployed: bool,
    pub install_dir: String,
    pub deploy_dir: String,
    pub compose_file: String,
    pub docker_available: bool,
    pub docker_running: bool,
    pub compose_available: bool,
    pub artifacts: Vec<RemoteArtifactStatus>,
    pub record_version: String,
    pub record_missing: bool,
    pub containers: Vec<ContainerStatusItem>,
    pub running_containers: usize,
    pub total_containers: usize,
    pub issues: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteUpdateProgress {
    pub step: String,
    pub percent: usize,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteUpdateResult {
    pub success: bool,
    pub components: Vec<String>,
    pub backup_dir: Option<String>,
    pub output: String,
    pub duration_secs: u64,
    pub status: Option<RemoteDeploymentStatus>,
}
