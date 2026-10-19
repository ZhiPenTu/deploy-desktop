use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentProfile {
    pub name: String,
    pub container_prefix: String,
    pub bundle_dir: String,
    pub deploy_dir: String,
    pub install_dir: String,
    pub backend_artifact: String,
    pub database: DatabaseProfile,
    pub containers: Vec<ContainerProfile>,
    pub images: Vec<ImageProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseProfile {
    pub service_key: String,
    pub name: String,
    pub client: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerProfile {
    pub service_key: String,
    pub label: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub protocol: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProfile {
    pub file: String,
    pub image_ref: String,
}

impl DeploymentProfile {
    pub fn container_name(&self, service_key: &str) -> String {
        format!("{}-{}", self.container_prefix, service_key)
    }

    pub fn database_container(&self) -> String {
        self.container_name(&self.database.service_key)
    }

    pub fn container_entries(&self) -> Vec<(String, String)> {
        self.containers
            .iter()
            .map(|container| {
                (
                    self.container_name(&container.service_key),
                    container.service_key.clone(),
                )
            })
            .collect()
    }

    pub fn service_key_for_container(&self, container: &str) -> String {
        let prefix = format!("{}-", self.container_prefix);
        container
            .strip_prefix(&prefix)
            .unwrap_or(container)
            .to_string()
    }

    pub fn port_definitions(&self) -> Vec<(u16, &str, String)> {
        self.containers
            .iter()
            .filter_map(|container| {
                container.port.map(|port| {
                    (
                        port,
                        container.protocol.as_str(),
                        format!("{} ({})", container.service_key, container.label),
                    )
                })
            })
            .collect()
    }
}

static PROFILE: OnceLock<DeploymentProfile> = OnceLock::new();

pub fn deployment_profile() -> &'static DeploymentProfile {
    PROFILE.get_or_init(|| {
        serde_json::from_str(include_str!("../../../profile.json"))
            .expect("profile.json 格式无效，请检查 JSON 字段")
    })
}

#[tauri::command]
pub fn get_deployment_profile() -> &'static DeploymentProfile {
    deployment_profile()
}
