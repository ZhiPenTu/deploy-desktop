//! 当前项目的部署配置视图。
//!
//! 项目拓扑来自交付包描述符，本模块只把当前项目与缓存交付包整理成
//! 各命令共用的只读结构，避免每个命令各自解析描述符。
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::bundle::Bundle;
use super::projects;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeploymentProfile {
    pub name: String,
    pub product: String,
    pub container_prefix: String,
    pub install_dir: String,
    /// 目标机上的 Compose 清单所在目录。
    pub deploy_dir: String,
    pub components: Vec<ComponentProfile>,
    pub database: Option<DatabaseProfile>,
    pub containers: Vec<ContainerProfile>,
    pub images: Vec<ImageProfile>,
    /// 依赖替换产物而非 Compose 重建的服务，更新后需要重启。
    pub restart_services: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ComponentProfile {
    pub key: String,
    pub label: String,
    pub services: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DatabaseProfile {
    pub service_key: String,
    pub name: String,
    pub client: String,
    pub port: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContainerProfile {
    pub service_key: String,
    pub label: String,
    pub port: Option<u16>,
    pub protocol: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageProfile {
    pub file: String,
    pub image_ref: String,
}

impl DeploymentProfile {
    pub fn component_services(&self, component: &str) -> Vec<String> {
        self.components
            .iter()
            .find(|item| item.key == component)
            .map(|item| item.services.clone())
            .unwrap_or_default()
    }

    pub fn component_label(&self, component: &str) -> String {
        self.components
            .iter()
            .find(|item| item.key == component)
            .map(|item| item.label.clone())
            .unwrap_or_else(|| component.to_string())
    }

    pub fn container_name(&self, service_key: &str) -> String {
        if self.container_prefix.is_empty() {
            service_key.to_string()
        } else {
            format!("{}-{}", self.container_prefix, service_key)
        }
    }

    pub fn database_container(&self) -> Option<String> {
        self.database
            .as_ref()
            .filter(|database| !database.service_key.trim().is_empty())
            .map(|database| self.container_name(&database.service_key))
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

    pub fn port_definitions(&self) -> Vec<(u16, String, String)> {
        self.containers
            .iter()
            .filter_map(|container| {
                container.port.map(|port| {
                    (
                        port,
                        container.protocol.clone(),
                        format!("{} ({})", container.service_key, container.label),
                    )
                })
            })
            .collect()
    }

    pub fn service_labels(&self) -> Vec<(String, String)> {
        self.containers
            .iter()
            .map(|container| (container.service_key.clone(), container.label.clone()))
            .collect()
    }
}

fn build(project: &projects::Project, bundle: Option<&Arc<Bundle>>) -> DeploymentProfile {
    let mut profile = DeploymentProfile {
        name: project.name.clone(),
        product: project.product.clone(),
        install_dir: project.install_dir.clone(),
        ..Default::default()
    };
    let Some(bundle) = bundle else {
        return profile;
    };
    let descriptor = &bundle.descriptor;
    profile.deploy_dir = std::path::Path::new(&project.install_dir)
        .join(bundle.deploy_source_rel())
        .to_string_lossy()
        .to_string();
    profile.product = descriptor.metadata.product.clone();
    profile.container_prefix = descriptor.runtime.container_prefix.trim().to_string();
    profile.components = descriptor
        .components
        .iter()
        .map(|component| ComponentProfile {
            key: component.key.clone(),
            label: component.label.clone(),
            services: component.services.clone(),
        })
        .collect();
    profile.database = descriptor
        .runtime
        .database
        .as_ref()
        .map(|database| DatabaseProfile {
            service_key: database.service_key.clone(),
            name: database.name.clone(),
            client: database.client.clone(),
            port: database.port,
        });
    profile.containers = descriptor
        .runtime
        .services
        .iter()
        .map(|service| ContainerProfile {
            service_key: service.key.clone(),
            label: service.label.clone(),
            port: service.port,
            protocol: if service.protocol.trim().is_empty() {
                "TCP".to_string()
            } else {
                service.protocol.clone()
            },
            description: service.description.clone(),
        })
        .collect();
    let compose_source = descriptor.runtime.compose.trim();
    let mut restart: Vec<String> = Vec::new();
    for component in &descriptor.components {
        let replaces_artifacts = component.artifacts.iter().any(|key| {
            descriptor
                .artifacts
                .iter()
                .find(|artifact| &artifact.key == key)
                .map(|artifact| artifact.source.trim() != compose_source)
                .unwrap_or(false)
        });
        if !replaces_artifacts {
            continue;
        }
        for service in &component.services {
            if !restart.contains(service) {
                restart.push(service.clone());
            }
        }
    }
    profile.restart_services = restart;
    profile.images = descriptor
        .images
        .iter()
        .map(|image| ImageProfile {
            file: std::path::Path::new(image.bundle_file.trim())
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
            image_ref: image.reference.clone(),
        })
        .collect();
    profile
}

/// 当前项目 + 缓存交付包整理出的只读配置；未选择项目或未缓存交付包时字段为空。
pub fn deployment_profile() -> DeploymentProfile {
    let Some(project) = projects::active_project() else {
        return DeploymentProfile::default();
    };
    let bundle = projects::active_bundle();
    build(&project, bundle.as_ref())
}

/// 数据库操作要求当前项目的交付包声明数据库。
pub fn require_database() -> Result<(DeploymentProfile, DatabaseProfile), String> {
    let profile = require_profile()?;
    let database = profile.database.clone().ok_or_else(|| {
        "当前项目的交付包未声明数据库，无法执行该操作".to_string()
    })?;
    Ok((profile, database))
}

/// 需要项目拓扑的命令使用该入口，缺失时给出可操作提示。
pub fn require_profile() -> Result<DeploymentProfile, String> {
    let Some(project) = projects::active_project() else {
        return Err("尚未选择项目，请先在侧边栏选择或新建项目".to_string());
    };
    let Some(bundle) = projects::active_bundle() else {
        return Err("当前项目还没有可用的交付包，请先在制品中心下载并选择版本".to_string());
    };
    Ok(build(&project, Some(&bundle)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::bundle::Bundle;

    fn bundle() -> Bundle {
        let path = std::env::temp_dir().join(format!(
            "profile-test-{}-{}",
            std::process::id(),
            crate::commands::bundle::sha256_hex(format!("{:?}", std::time::SystemTime::now()).as_bytes())
        ));
        std::fs::create_dir_all(path.join("app")).unwrap();
        std::fs::write(path.join("app/web"), "index").unwrap();
        let content = std::fs::read(path.join("app/web")).unwrap();
        let manifest = format!(
            r#"{{"apiVersion":"deploy.desktop/bundle/v2","kind":"DeploymentBundle",
"metadata":{{"product":"demo","version":"1-a","commit":"","commitShort":"a","branch":"main","builtAt":""}},
"runtime":{{"type":"compose","compose":"deploy/docker-compose.yml","containerPrefix":"demo",
"services":[{{"key":"web","label":"Web","port":80,"protocol":"TCP","description":""}},{{"key":"db","label":"数据库","port":5432,"protocol":"TCP","description":""}}],
"database":{{"serviceKey":"db","name":"demo","client":"psql","port":5432}}}},
"components":[{{"key":"web","label":"前端","artifacts":["site"],"services":["web"]}}],
"artifacts":[{{"key":"site","label":"站点","source":"app","root":"deploy","path":"site","type":"dir"}},
{{"key":"compose","label":"配置","source":"deploy/docker-compose.yml","root":"deploy","path":"docker-compose.yml","type":"file"}}],
"images":[],"plan":{{"steps":[]}},
"files":[{{"path":"app/web","size":{},"sha256":"{}"}},{{"path":"deploy/docker-compose.yml","size":5,"sha256":"{}"}}]}}"#,
            content.len(),
            crate::commands::bundle::sha256_hex(&content),
            crate::commands::bundle::sha256_hex(b"a: 1\n")
        );
        std::fs::create_dir_all(path.join("deploy")).unwrap();
        std::fs::write(path.join("deploy/docker-compose.yml"), "a: 1\n").unwrap();
        std::fs::write(path.join("bundle.json"), manifest).unwrap();
        Bundle::load(&path).unwrap()
    }

    #[test]
    fn profile_is_built_from_descriptor() {
        let bundle = std::sync::Arc::new(bundle());
        let project = projects::Project {
            id: "p1".to_string(),
            name: "演示".to_string(),
            install_dir: "/srv/demo".to_string(),
            ..Default::default()
        };
        let profile = build(&project, Some(&bundle));
        assert_eq!(profile.container_prefix, "demo");
        assert_eq!(profile.container_name("web"), "demo-web");
        assert_eq!(profile.component_services("web"), vec!["web"]);
        assert_eq!(profile.database_container().as_deref(), Some("demo-db"));
    }
}
