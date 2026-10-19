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
    #[serde(default)]
    pub components: ComponentProfile,
    pub database: DatabaseProfile,
    pub containers: Vec<ContainerProfile>,
    pub images: Vec<ImageProfile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentProfile {
    #[serde(default)]
    pub backend: Vec<String>,
    #[serde(default)]
    pub gateway: Vec<String>,
    #[serde(default)]
    pub frontend: Vec<String>,
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
    pub fn component_services(&self, component: &str) -> Vec<String> {
        let configured = match component {
            "backend" => &self.components.backend,
            "gateway" => &self.components.gateway,
            "frontend" => &self.components.frontend,
            _ => return Vec::new(),
        };
        if !configured.is_empty() {
            return configured.clone();
        }
        // 兼容未声明 components 的旧 profile，保持原有更新行为。
        match component {
            "backend" => vec!["backend-java".to_string(), "backend-java-b".to_string()],
            "gateway" => vec!["gateway-shard-1".to_string(), "gateway-shard-2".to_string()],
            "frontend" => vec!["frontend".to_string()],
            _ => Vec::new(),
        }
    }

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

const DEFAULT_PROFILE: &str = include_str!("../../../profile.example.json");

fn parse_profile(content: &str) -> Option<DeploymentProfile> {
    serde_json::from_str(content).ok()
}

pub fn init_profile(app: &tauri::AppHandle) {
    use tauri::Manager;
    let mut loaded = None;
    if let Ok(dir) = app.path().app_config_dir() {
        let path = dir.join("profile.json");
        if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                loaded = parse_profile(&content);
            }
        }
    }
    let profile = loaded
        .or_else(|| parse_profile(DEFAULT_PROFILE))
        .expect("内置部署配置示例格式无效");
    let _ = PROFILE.set(profile);
}

pub fn deployment_profile() -> &'static DeploymentProfile {
    PROFILE
        .get()
        .unwrap_or_else(|| {
            PROFILE.get_or_init(|| {
                parse_profile(DEFAULT_PROFILE).expect("内置部署配置示例格式无效")
            })
        })
}

#[tauri::command]
pub fn get_deployment_profile() -> &'static DeploymentProfile {
    deployment_profile()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_with(components: ComponentProfile) -> DeploymentProfile {
        DeploymentProfile {
            name: "test".to_string(),
            container_prefix: "demo".to_string(),
            bundle_dir: String::new(),
            deploy_dir: String::new(),
            install_dir: String::new(),
            backend_artifact: "app.jar".to_string(),
            components,
            database: DatabaseProfile {
                service_key: "database".to_string(),
                name: "demo".to_string(),
                client: "psql".to_string(),
                port: 5432,
            },
            containers: Vec::new(),
            images: Vec::new(),
        }
    }

    #[test]
    fn configured_components_take_precedence() {
        let profile = profile_with(ComponentProfile {
            backend: vec!["api".to_string()],
            gateway: vec!["gw-a".to_string(), "gw-b".to_string()],
            frontend: vec!["web".to_string()],
        });
        assert_eq!(profile.component_services("backend"), vec!["api"]);
        assert_eq!(profile.component_services("gateway"), vec!["gw-a", "gw-b"]);
        assert_eq!(profile.component_services("frontend"), vec!["web"]);
        assert!(profile.component_services("unknown").is_empty());
    }

    #[test]
    fn missing_components_fall_back_to_legacy_defaults() {
        let profile = profile_with(ComponentProfile::default());
        assert_eq!(profile.component_services("frontend"), vec!["frontend"]);
        assert_eq!(profile.component_services("gateway").len(), 2);
    }
}
