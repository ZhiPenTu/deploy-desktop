use crate::commands::docker_ops::ContainerStatusItem;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteArtifactInfo {
    pub exists: bool,
    pub size_bytes: u64,
    pub modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDeploymentStatus {
    pub deployed: bool,
    pub install_dir: String,
    pub deploy_dir: String,
    pub compose_file: String,
    pub docker_available: bool,
    pub docker_running: bool,
    pub compose_available: bool,
    pub server_jar: RemoteArtifactInfo,
    pub gateway_classes: RemoteArtifactInfo,
    pub gateway_lib: RemoteArtifactInfo,
    pub frontend_dist: RemoteArtifactInfo,
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

pub(super) struct BundlePaths {
    pub(super) server_jar: Option<PathBuf>,
    pub(super) gateway_classes: Option<PathBuf>,
    pub(super) gateway_lib: Option<PathBuf>,
    pub(super) frontend_dist: Option<PathBuf>,
    pub(super) compose_file: Option<PathBuf>,
    pub(super) compose_lan_file: Option<PathBuf>,
    pub(super) nginx_dir: Option<PathBuf>,
    pub(super) oceanbase_dir: Option<PathBuf>,
    pub(super) image_files: Vec<PathBuf>,
}
