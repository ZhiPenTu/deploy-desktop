use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

use super::image_library::{login_docker, TempDockerConfig};

const SOURCES_FILE: &str = "registry-sources.json";
const SECRETS_FILE: &str = "registry-secrets.json";
static STORE_LOCK: Mutex<()> = Mutex::new(());

// 首次使用时预置的国内公共加速镜像源；保存后与用户自行添加的源一致，可编辑或删除。
const DEFAULT_SOURCES: &[(&str, &str, &str)] = &[
    ("default-daocloud", "DaoCloud 加速", "docker.m.daocloud.io"),
    ("default-1ms", "毫秒镜像加速", "docker.1ms.run"),
    ("default-xuanyuan", "轩辕镜像加速", "docker.xuanyuan.me"),
];

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RegistrySource {
    pub id: String,
    pub name: String,
    pub address: String,
    pub namespace: String,
    pub username: String,
    pub has_password: bool,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrySourceInput {
    pub id: String,
    pub name: String,
    pub address: String,
    pub namespace: String,
    pub username: String,
    pub password: Option<String>,
    pub clear_password: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryTestResult {
    pub authenticated: bool,
    pub message: String,
}

pub(super) struct RegistrySourceAccess {
    pub source: RegistrySource,
    pub password: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistrySecret {
    address: String,
    username: String,
    password: String,
}

impl RegistrySecret {
    fn matches(&self, source: &RegistrySource) -> bool {
        self.address == source.address && self.username == source.username
    }
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|_| "无法读取镜像源配置目录".to_string())?;
    fs::create_dir_all(&dir).map_err(|_| "无法创建镜像源配置目录，请检查目录权限".to_string())?;
    Ok(dir)
}

fn read_sources(dir: &Path) -> Result<Vec<RegistrySource>, String> {
    read_json(&dir.join(SOURCES_FILE), "镜像源配置")
}

fn default_sources() -> Vec<RegistrySource> {
    DEFAULT_SOURCES
        .iter()
        .map(|(id, name, address)| RegistrySource {
            id: id.to_string(),
            name: name.to_string(),
            address: address.to_string(),
            namespace: "library".to_string(),
            ..Default::default()
        })
        .collect()
}

// 仅在配置文件尚不存在时写入默认源；用户删空后不再重新生成。
fn read_sources_with_defaults(dir: &Path) -> Result<Vec<RegistrySource>, String> {
    let path = dir.join(SOURCES_FILE);
    if !path.exists() {
        write_private_json(&path, &default_sources())?;
    }
    read_sources(dir)
}

fn read_json<T: serde::de::DeserializeOwned + Default>(
    path: &Path,
    label: &str,
) -> Result<T, String> {
    match fs::read_to_string(path) {
        Ok(content) => {
            serde_json::from_str(&content).map_err(|_| format!("{label}已损坏，请恢复本机配置备份"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(_) => Err(format!("无法读取{label}，请检查目录权限")),
    }
}

fn write_private_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let content = serde_json::to_vec_pretty(value).map_err(|_| "无法保存镜像源配置".to_string())?;
    let temporary = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| "无法保存镜像源配置，请检查目录权限".to_string())?;
    file.write_all(&content)
        .and_then(|_| file.sync_all())
        .map_err(|_| "写入镜像源配置失败，请检查磁盘空间".to_string())?;
    drop(file);
    fs::rename(&temporary, path).map_err(|_| "更新镜像源配置失败，请检查目录权限".to_string())
}

pub(super) fn normalize_address(raw: &str) -> Result<String, String> {
    let value = raw
        .trim()
        .strip_prefix("https://")
        .unwrap_or(raw.trim())
        .trim_end_matches('/');
    if value.is_empty()
        || value.starts_with('-')
        || value.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | ':' | '[' | ']'))
        })
    {
        return Err("请填写有效的镜像仓库地址，仅包含主机名和可选端口".to_string());
    }
    let url = reqwest::Url::parse(&format!("https://{value}"))
        .map_err(|_| "镜像仓库地址或端口无效".to_string())?;
    let host = url
        .host_str()
        .ok_or_else(|| "请填写镜像仓库主机名".to_string())?;
    if host != "localhost" && !host.contains('.') && !value.contains(':') {
        return Err("仓库主机名不含点时，请同时填写端口，例如 harbor:5000".to_string());
    }
    if url.port() == Some(0) {
        return Err("镜像仓库端口需在 1 到 65535 之间".to_string());
    }
    Ok(value.to_ascii_lowercase())
}

fn normalize_namespace(raw: &str) -> Result<String, String> {
    let namespace = raw.trim();
    if !namespace.is_empty()
        && (namespace.contains("..")
            || namespace
                .split('/')
                .any(|part| part.is_empty() || part == ".")
            || namespace.chars().any(|character| {
                !(character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '.' | '_' | '-' | '/'))
            }))
    {
        return Err("命名空间只能包含小写字母、数字、点、下划线、短横线和分级斜杠".to_string());
    }
    Ok(namespace.to_string())
}

fn save_source_at(dir: &Path, input: RegistrySourceInput) -> Result<RegistrySource, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() || name.len() > 160 || name.chars().any(char::is_control) {
        return Err("请填写镜像源名称，长度不超过 160 字节".to_string());
    }
    let address = normalize_address(&input.address)?;
    let namespace = normalize_namespace(&input.namespace)?;
    let username = input.username.trim().to_string();
    if username.starts_with('-')
        || username
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err("仓库用户名不能包含空格、控制字符或以短横线开头".to_string());
    }
    let mut sources = read_sources(dir)?;
    let existing = sources.iter().position(|source| source.id == input.id);
    if !input.id.is_empty() && existing.is_none() {
        return Err("镜像源不存在或已被删除，请刷新后重试".to_string());
    }
    if sources
        .iter()
        .any(|source| source.id != input.id && source.name.to_lowercase() == name.to_lowercase())
    {
        return Err("已存在同名镜像源，请使用其他名称".to_string());
    }
    let id = existing
        .map(|index| sources[index].id.clone())
        .unwrap_or_else(|| {
            format!(
                "registry-{:x}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            )
        });
    let mut secrets: BTreeMap<String, RegistrySecret> =
        read_json(&dir.join(SECRETS_FILE), "镜像源凭据")?;
    let identity_changed = existing.is_some_and(|index| {
        sources[index].address != address || sources[index].username != username
    });
    if input.clear_password || identity_changed || username.is_empty() {
        secrets.remove(&id);
    }
    if !input.clear_password {
        if let Some(password) = input.password.filter(|password| !password.is_empty()) {
            if username.is_empty() {
                return Err("已填写仓库密码，请同时填写仓库用户名".to_string());
            }
            secrets.insert(
                id.clone(),
                RegistrySecret {
                    address: address.clone(),
                    username: username.clone(),
                    password,
                },
            );
        }
    }
    let record = RegistrySource {
        id: id.clone(),
        name,
        address,
        namespace,
        username,
        has_password: secrets.contains_key(&id),
    };
    match existing {
        Some(index) => sources[index] = record.clone(),
        None => sources.push(record.clone()),
    }
    write_private_json(&dir.join(SECRETS_FILE), &secrets)?;
    write_private_json(&dir.join(SOURCES_FILE), &sources)?;
    Ok(record)
}

fn delete_source_at(dir: &Path, id: &str) -> Result<(), String> {
    let mut sources = read_sources(dir)?;
    if !sources.iter().any(|source| source.id == id) {
        return Err("镜像源不存在或已被删除".to_string());
    }
    let mut secrets: BTreeMap<String, RegistrySecret> =
        read_json(&dir.join(SECRETS_FILE), "镜像源凭据")?;
    sources.retain(|source| source.id != id);
    secrets.remove(id);
    write_private_json(&dir.join(SECRETS_FILE), &secrets)?;
    write_private_json(&dir.join(SOURCES_FILE), &sources)
}

fn access_at(dir: &Path, id: &str) -> Result<RegistrySourceAccess, String> {
    let source = read_sources(dir)?
        .into_iter()
        .find(|source| source.id == id)
        .ok_or_else(|| "所选镜像源已被删除，请重新选择".to_string())?;
    let mut secrets: BTreeMap<String, RegistrySecret> =
        read_json(&dir.join(SECRETS_FILE), "镜像源凭据")?;
    let password = secrets
        .remove(id)
        .filter(|secret| secret.matches(&source))
        .map(|secret| secret.password);
    Ok(RegistrySourceAccess { source, password })
}

pub(super) fn source_access(
    app: &AppHandle,
    id: Option<&str>,
) -> Result<Option<RegistrySourceAccess>, String> {
    let Some(id) = id.filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "镜像源配置暂不可用，请重新打开应用".to_string())?;
    access_at(&config_dir(app)?, id).map(Some)
}

pub(super) fn reference_from_source(
    source: &RegistrySource,
    reference: &str,
) -> Result<String, String> {
    let reference = reference.trim();
    let first = reference.split('/').next().unwrap_or("");
    if reference.contains('/')
        && (first.contains('.') || first.contains(':') || first == "localhost")
    {
        if first != source.address {
            return Err(
                "镜像地址与所选镜像源不一致，请更换镜像源或填写仓库内的相对路径".to_string(),
            );
        }
        return Ok(reference.to_string());
    }
    let prefix = if source.namespace.is_empty() {
        source.address.clone()
    } else {
        format!("{}/{}", source.address, source.namespace)
    };
    Ok(format!("{prefix}/{reference}"))
}

pub(super) fn source_credentials(
    access: RegistrySourceAccess,
    password: Option<String>,
) -> Result<(Option<String>, Option<String>), String> {
    let username = access.source.username;
    if username.is_empty() {
        return Ok((None, None));
    }
    let password = password
        .filter(|password| !password.is_empty())
        .or(access.password)
        .ok_or_else(|| "此镜像源未保存密码，请填写本次密码或编辑镜像源".to_string())?;
    Ok((Some(username), Some(password)))
}

#[tauri::command]
pub fn list_registry_sources(app: AppHandle) -> Result<Vec<RegistrySource>, String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "镜像源配置暂不可用".to_string())?;
    let dir = config_dir(&app)?;
    let mut sources = read_sources_with_defaults(&dir)?;
    let secrets: BTreeMap<String, RegistrySecret> =
        read_json(&dir.join(SECRETS_FILE), "镜像源凭据")?;
    for source in &mut sources {
        source.has_password = secrets
            .get(&source.id)
            .is_some_and(|secret| secret.matches(source));
    }
    Ok(sources)
}

#[tauri::command]
pub fn save_registry_source(
    app: AppHandle,
    source: RegistrySourceInput,
) -> Result<RegistrySource, String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "镜像源配置暂不可用".to_string())?;
    let label = source.name.clone();
    let result = save_source_at(&config_dir(&app)?, source);
    match &result {
        Ok(saved) => log::info!(target: "registry", "保存镜像源：{} {}", saved.name, saved.address),
        Err(error) => log::error!(target: "registry", "保存镜像源失败：{label}：{error}"),
    }
    result
}

#[tauri::command]
pub fn delete_registry_source(app: AppHandle, id: String) -> Result<(), String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "镜像源配置暂不可用".to_string())?;
    let result = delete_source_at(&config_dir(&app)?, &id);
    match &result {
        Ok(_) => log::info!(target: "registry", "删除镜像源：{id}"),
        Err(error) => log::error!(target: "registry", "删除镜像源失败：{id}：{error}"),
    }
    result
}

#[tauri::command]
pub async fn test_registry_source(
    app: AppHandle,
    id: String,
) -> Result<RegistryTestResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let access = source_access(&app, Some(&id))?.ok_or_else(|| "请选择镜像源".to_string())?;
        if !access.source.username.is_empty() {
            let address = access.source.address.clone();
            let (username, password) = source_credentials(access, None)?;
            let config = TempDockerConfig::create()?;
            login_docker(&address, &username.unwrap_or_default(), &password.unwrap_or_default(), &config.path)?;
            return Ok(RegistryTestResult { authenticated: true, message: "镜像源登录验证通过；镜像访问权限以实际拉取结果为准".to_string() });
        }
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none()).build().map_err(|_| "无法创建镜像源连接".to_string())?;
        let response = client.get(format!("https://{}/v2/", access.source.address)).send()
            .map_err(|_| "无法连接镜像源，请检查仓库地址、网络和证书；HTTP 仓库请先配置 Docker 后填写登录凭据测试".to_string())?;
        let registry_v2 = response.headers().get("docker-distribution-api-version").is_some_and(|value| value == "registry/2.0");
        match response.status().as_u16() {
            200 if registry_v2 =>
                Ok(RegistryTestResult { authenticated: false, message: "镜像源可连接；镜像访问权限以实际拉取结果为准".to_string() }),
            // 公共加速仓库对 /v2/ 返回带 Registry V2 头的 401 匿名挑战，属于正常响应。
            401 if registry_v2 =>
                Ok(RegistryTestResult { authenticated: false, message: "镜像源可连接，公开镜像可直接拉取；拉取私有镜像请在镜像源中填写用户名和密码".to_string() }),
            401 => Err("镜像源可连接，但需要登录，请编辑镜像源并填写用户名和密码".to_string()),
            _ => Err("镜像源未返回有效的仓库响应，请检查仓库地址和访问权限".to_string()),
        }
    }).await.map_err(|_| "镜像源测试任务失败，请重试".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> RegistrySourceInput {
        RegistrySourceInput {
            name: "测试镜像源".to_string(),
            address: "registry.example.com:5000".to_string(),
            namespace: "demo".to_string(),
            username: "reader".to_string(),
            password: Some("test-only-password".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn seeds_default_sources_once_and_keeps_edits_and_deletions() {
        let dir = std::env::temp_dir().join(format!(
            "registry-defaults-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let seeded = read_sources_with_defaults(&dir).unwrap();
        assert_eq!(seeded.len(), DEFAULT_SOURCES.len());
        let mut ids = seeded.iter().map(|source| source.id.clone()).collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), seeded.len());
        for source in &seeded {
            assert!(normalize_address(&source.address).is_ok());
            assert_eq!(source.namespace, "library");
        }
        let edited = save_source_at(
            &dir,
            RegistrySourceInput {
                id: seeded[0].id.clone(),
                name: "团队加速源".to_string(),
                address: seeded[0].address.clone(),
                namespace: "mirror".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(edited.id, seeded[0].id);
        assert_eq!(edited.namespace, "mirror");
        assert_eq!(
            read_sources_with_defaults(&dir).unwrap().len(),
            DEFAULT_SOURCES.len()
        );
        for source in read_sources_with_defaults(&dir).unwrap() {
            delete_source_at(&dir, &source.id).unwrap();
        }
        assert!(read_sources_with_defaults(&dir).unwrap().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn private_source_lifecycle_and_credential_identity() {
        let dir = std::env::temp_dir().join(format!(
            "registry-sources-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let saved = save_source_at(&dir, input()).unwrap();
        assert!(saved.has_password);
        assert!(!fs::read_to_string(dir.join(SOURCES_FILE))
            .unwrap()
            .contains("test-only-password"));
        assert!(!serde_json::to_string(&saved)
            .unwrap()
            .contains("test-only-password"));
        assert_eq!(
            access_at(&dir, &saved.id).unwrap().password.as_deref(),
            Some("test-only-password")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir.join(SECRETS_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let edited = save_source_at(
            &dir,
            RegistrySourceInput {
                id: saved.id.clone(),
                password: None,
                ..input()
            },
        )
        .unwrap();
        assert!(edited.has_password);
        let changed = save_source_at(
            &dir,
            RegistrySourceInput {
                id: saved.id.clone(),
                address: "other.example.com".to_string(),
                password: None,
                ..input()
            },
        )
        .unwrap();
        assert!(!changed.has_password);
        assert!(source_credentials(access_at(&dir, &saved.id).unwrap(), None).is_err());
        delete_source_at(&dir, &saved.id).unwrap();
        assert!(read_sources(&dir).unwrap().is_empty());
        assert!(!fs::read_to_string(dir.join(SECRETS_FILE))
            .unwrap()
            .contains("test-only-password"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn validates_addresses_and_prevents_cross_registry_credentials() {
        assert_eq!(
            normalize_address("https://REGISTRY.example.com:5000/").unwrap(),
            "registry.example.com:5000"
        );
        for address in [
            "",
            "http://example.com",
            "example.com/path",
            "user:secret@example.com",
            "example.com?token=secret",
            "-host",
            "harbor",
            "example.com:0",
            "example.com:65536",
        ] {
            assert!(normalize_address(address).is_err(), "{address}");
        }
        assert!(normalize_address("localhost:5000").is_ok());
        let source = RegistrySource {
            address: "registry.example.com".to_string(),
            namespace: "demo".to_string(),
            ..Default::default()
        };
        assert_eq!(
            reference_from_source(&source, "app@sha256:abc").unwrap(),
            "registry.example.com/demo/app@sha256:abc"
        );
        assert!(reference_from_source(&source, "other.example.com/app@sha256:abc").is_err());
        assert!(normalize_namespace("../demo").is_err());
        assert!(normalize_namespace("demo//app").is_err());
    }

    #[test]
    fn credentials_stay_bound_to_the_saved_identity_after_partial_write() {
        let dir = std::env::temp_dir().join(format!(
            "registry-identity-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let saved = save_source_at(&dir, input()).unwrap();
        let mut secrets: BTreeMap<String, RegistrySecret> =
            read_json(&dir.join(SECRETS_FILE), "镜像源凭据").unwrap();
        secrets.get_mut(&saved.id).unwrap().address = "different.example.com".to_string();
        write_private_json(&dir.join(SECRETS_FILE), &secrets).unwrap();
        assert!(access_at(&dir, &saved.id).unwrap().password.is_none());
        let restored = save_source_at(
            &dir,
            RegistrySourceInput {
                id: saved.id.clone(),
                ..input()
            },
        )
        .unwrap();
        assert!(restored.has_password);
        let cleared = save_source_at(
            &dir,
            RegistrySourceInput {
                id: saved.id.clone(),
                clear_password: true,
                password: None,
                ..input()
            },
        )
        .unwrap();
        assert!(!cleared.has_password);
        assert!(source_credentials(
            access_at(&dir, &saved.id).unwrap(),
            Some("one-time-only".to_string())
        )
        .is_ok());
        assert!(access_at(&dir, &saved.id).unwrap().password.is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
