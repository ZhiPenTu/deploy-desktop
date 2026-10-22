//! Compose 清单读取。
//!
//! 部署预检、部署复核与远程运维都要从同一份 `docker-compose*.yml` 读出服务与镜像。
//! 文件名候选与字段解析集中在这里，避免一处改了顺序，另一处仍按旧规则找文件。
//!
//! 这里保留两种读取方式：`parse_services` 用 YAML 解析，供需要完整服务信息的场景；
//! `scan_image_references` 只按行寻找 `image:`，用于清单结构不完整时仍要给出镜像
//! 准备清单的场景。两者都不访问磁盘。

use serde_yaml::Value;

/// 部署目录根下可直接使用的清单文件名，按优先级排列。
pub const ROOT_CANDIDATES: [&str; 2] = ["docker-compose.lan.yml", "docker-compose.yml"];

/// 连同 `deploy/` 子目录一起搜索时的候选清单，按优先级排列。
pub const SEARCH_CANDIDATES: [&str; 4] = [
    ROOT_CANDIDATES[0],
    ROOT_CANDIDATES[1],
    "deploy/docker-compose.lan.yml",
    "deploy/docker-compose.yml",
];

/// 解析出的单个服务。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub image: String,
    pub container_name: String,
    pub restart: String,
    pub ports: Vec<String>,
    pub depends_on: Vec<String>,
    pub profiles: Vec<String>,
}

/// 解析失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// YAML 语法错误，附带解析器给出的位置信息。
    Yaml(String),
    /// 文档里没有可用的 `services` 映射。
    MissingServices,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Yaml(detail) => write!(formatter, "YAML 解析失败：{detail}"),
            Error::MissingServices => write!(formatter, "未声明 services"),
        }
    }
}

/// 读取 `services` 下的每个服务。服务顺序与清单中的顺序一致。
pub fn parse_services(content: &str) -> Result<Vec<Service>, Error> {
    let document: Value =
        serde_yaml::from_str(content).map_err(|error| Error::Yaml(error.to_string()))?;
    let Some(services) = document.get("services").and_then(Value::as_mapping) else {
        return Err(Error::MissingServices);
    };
    Ok(services
        .iter()
        .filter_map(|(key, body)| {
            let name = key.as_str()?;
            Some(Service {
                name: name.to_string(),
                image: yaml_text(body.get("image")),
                container_name: yaml_text(body.get("container_name")),
                restart: yaml_text(body.get("restart")),
                ports: yaml_list(body.get("ports")),
                depends_on: depends_on(body.get("depends_on")),
                profiles: yaml_list(body.get("profiles")),
            })
        })
        .collect())
}

/// 只取服务名。
pub fn service_names(content: &str) -> Result<Vec<String>, Error> {
    Ok(parse_services(content)?
        .into_iter()
        .map(|service| service.name)
        .collect())
}

/// 按行扫描 `image:`，保留出现顺序与重复项，由调用方决定去重。
///
/// 用于清单结构不足以完整解析、但仍需要镜像清单的场景。
pub fn scan_image_references(content: &str) -> Vec<String> {
    let mut references = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("image:") else {
            continue;
        };
        let reference = rest.trim().trim_matches('"').trim_matches('\'').to_string();
        if !reference.is_empty() {
            references.push(reference);
        }
    }
    references
}

fn yaml_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

fn yaml_list(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Sequence(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| yaml_text(Some(item)))
        .filter(|text| !text.is_empty())
        .collect()
}

fn depends_on(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Mapping(map)) => map
            .keys()
            .filter_map(|key| key.as_str().map(str::to_string))
            .collect(),
        Some(Value::Sequence(items)) => items
            .iter()
            .map(|item| yaml_text(Some(item)))
            .filter(|text| !text.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "services:\n  web:\n    image: nginx:1.27\n    container_name: demo-web\n    restart: unless-stopped\n    ports:\n      - 8080:80\n    depends_on:\n      api: { condition: service_healthy }\n    profiles: [main]\n  api:\n    image: 'demo/api:2'\n    depends_on: [db]\n";

    #[test]
    fn parses_services_in_declaration_order() {
        let services = parse_services(FULL).unwrap();
        assert_eq!(
            services
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            vec!["web", "api"]
        );
        let web = &services[0];
        assert_eq!(web.image, "nginx:1.27");
        assert_eq!(web.container_name, "demo-web");
        assert_eq!(web.restart, "unless-stopped");
        assert_eq!(web.ports, vec!["8080:80"]);
        assert_eq!(web.depends_on, vec!["api"]);
        assert_eq!(web.profiles, vec!["main"]);
        assert_eq!(services[1].image, "demo/api:2");
        assert_eq!(services[1].depends_on, vec!["db"]);
    }

    #[test]
    fn missing_services_and_invalid_yaml_are_distinguished() {
        assert_eq!(
            parse_services("version: '3'\n"),
            Err(Error::MissingServices)
        );
        assert!(matches!(
            parse_services("services: [\n"),
            Err(Error::Yaml(_))
        ));
        assert_eq!(
            service_names("services: {}\n").unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn image_scan_ignores_comments_and_blank_values() {
        let content = "# image: commented\nservices:\n  web:\n    image: \"docker.io/library/nginx:1.27\"\n  api:\n    image:\n  db:\n    image: 'redis:7'\n    # image: nested\n";
        assert_eq!(
            scan_image_references(content),
            vec!["docker.io/library/nginx:1.27", "redis:7"]
        );
    }

    #[test]
    fn candidate_lists_keep_lan_variant_first() {
        assert_eq!(SEARCH_CANDIDATES.len(), 4);
        assert!(SEARCH_CANDIDATES[0].ends_with("lan.yml"));
        assert_eq!(
            &SEARCH_CANDIDATES[SEARCH_CANDIDATES.len() - 1],
            &"deploy/docker-compose.yml"
        );
        assert_eq!(
            ROOT_CANDIDATES,
            ["docker-compose.lan.yml", "docker-compose.yml"]
        );
    }
}
