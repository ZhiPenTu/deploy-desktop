use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use super::remote_ssh::{setup_remote_ssh_key, test_ssh_connection, SshTestResult};

const CONNECTIONS_FILE: &str = "connections.json";
const SECRETS_FILE: &str = "connection-secrets.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SshCredential {
    pub id: String,
    pub name: String,
    pub username: String,
    pub private_key_path: String,
    pub has_password: bool,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SshCredentialInput {
    pub id: String,
    pub name: String,
    pub username: String,
    pub private_key_path: String,
    pub password: Option<String>,
    pub clear_password: bool,
}

fn default_ssh_port() -> u16 {
    22
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedHost {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub credential_id: String,
    pub deploy_dir: String,
    pub notes: String,
    pub updated_at: u64,
}

impl Default for ManagedHost {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            address: String::new(),
            port: default_ssh_port(),
            credential_id: String::new(),
            deploy_dir: String::new(),
            notes: String::new(),
            updated_at: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedHostInput {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub credential_id: String,
    pub deploy_dir: String,
    pub notes: String,
}

impl ManagedHostInput {
    fn with_default_port(mut self) -> Self {
        if self.port == 0 {
            self.port = default_ssh_port();
        }
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ConnectionsFile {
    credentials: Vec<SshCredential>,
    hosts: Vec<ManagedHost>,
    active_host_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInventory {
    pub credentials: Vec<SshCredential>,
    pub hosts: Vec<ManagedHost>,
    pub active_host_id: String,
}

type SecretStore = BTreeMap<String, String>;

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("读取应用配置目录失败：{e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建应用配置目录失败：{e}"))?;
    Ok(dir)
}

fn connections_path(dir: &Path) -> PathBuf {
    dir.join(CONNECTIONS_FILE)
}

fn secrets_path(dir: &Path) -> PathBuf {
    dir.join(SECRETS_FILE)
}

fn write_private_file(path: &Path, content: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("写入本地连接配置失败：{e}"))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("写入本地连接配置失败：{e}"))?;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        fs::write(path, content).map_err(|e| format!("写入本地连接配置失败：{e}"))?;
    }
    Ok(())
}

fn load_store(dir: &Path) -> Result<ConnectionsFile, String> {
    let path = connections_path(dir);
    if !path.exists() {
        return Ok(ConnectionsFile::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读取连接配置失败：{e}"))?;
    let mut store: ConnectionsFile =
        serde_json::from_str(&content).map_err(|e| format!("解析连接配置失败：{e}"))?;

    if !store.active_host_id.is_empty()
        && !store
            .hosts
            .iter()
            .any(|host| host.id == store.active_host_id)
    {
        store.active_host_id.clear();
    }
    if store.active_host_id.is_empty() {
        if let Some(host) = store.hosts.first() {
            store.active_host_id = host.id.clone();
        }
    }
    Ok(store)
}

fn store_connections(dir: &Path, store: &ConnectionsFile) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(store).map_err(|e| format!("保存连接配置失败：{e}"))?;
    write_private_file(&connections_path(dir), &content)
}

fn read_secrets(dir: &Path) -> Result<SecretStore, String> {
    let path = secrets_path(dir);
    if !path.exists() {
        return Ok(SecretStore::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读取本地凭证失败：{e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("解析本地凭证失败：{e}"))
}

fn write_secrets(dir: &Path, secrets: &SecretStore) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(secrets).map_err(|e| format!("保存本地凭证失败：{e}"))?;
    write_private_file(&secrets_path(dir), &content)
}

fn inventory_at(dir: &Path) -> Result<ConnectionInventory, String> {
    let mut store = load_store(dir)?;
    let secrets = read_secrets(dir)?;
    for credential in &mut store.credentials {
        credential.has_password = secrets.contains_key(&credential.id);
    }
    Ok(ConnectionInventory {
        credentials: store.credentials,
        hosts: store.hosts,
        active_host_id: store.active_host_id,
    })
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn next_id(prefix: &str, used: impl Fn(&str) -> bool) -> String {
    let base = format!("{prefix}-{:x}", now_millis());
    if !used(&base) {
        return base;
    }
    let mut sequence = 1;
    loop {
        let candidate = format!("{base}-{sequence}");
        if !used(&candidate) {
            return candidate;
        }
        sequence += 1;
    }
}

fn same_name(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

fn save_credential_at(dir: &Path, input: SshCredentialInput) -> Result<SshCredential, String> {
    let name = input.name.trim().to_string();
    let username = input.username.trim().to_string();
    if name.is_empty() {
        return Err("请填写凭证名称".to_string());
    }
    if username.is_empty() {
        return Err("请填写登录用户名".to_string());
    }
    if username.starts_with('-') || username.chars().any(char::is_whitespace) {
        return Err("登录用户名不能包含空格或以 - 开头".to_string());
    }

    let mut store = load_store(dir)?;
    let requested_id = input.id.trim().to_string();
    let existing = if requested_id.is_empty() {
        None
    } else {
        store
            .credentials
            .iter()
            .position(|item| item.id == requested_id)
    };
    if store.credentials.iter().any(|item| {
        Some(item.id.as_str()) != existing.map(|index| store.credentials[index].id.as_str())
            && same_name(&item.name, &name)
    }) {
        return Err(format!("已存在名为「{name}」的凭证"));
    }

    let id = if requested_id.is_empty() {
        next_id("cred", |candidate| {
            store.credentials.iter().any(|item| item.id == candidate)
        })
    } else {
        requested_id
    };

    let mut secrets = read_secrets(dir)?;
    if input.clear_password {
        secrets.remove(&id);
    } else if let Some(password) = input.password.filter(|value| !value.is_empty()) {
        secrets.insert(id.clone(), password);
    }
    write_secrets(dir, &secrets)?;

    let record = SshCredential {
        id: id.clone(),
        name,
        username,
        private_key_path: input.private_key_path.trim().to_string(),
        has_password: secrets.contains_key(&id),
        updated_at: now_millis(),
    };
    match existing {
        Some(index) => store.credentials[index] = record.clone(),
        None => store.credentials.push(record.clone()),
    }
    store_connections(dir, &store)?;
    Ok(record)
}

fn delete_credential_at(dir: &Path, id: &str) -> Result<(), String> {
    let mut store = load_store(dir)?;
    let Some(credential) = store.credentials.iter().find(|item| item.id == id) else {
        return Err("凭证不存在或已被删除".to_string());
    };
    let names: Vec<&str> = store
        .hosts
        .iter()
        .filter(|host| host.credential_id == id)
        .map(|host| host.name.as_str())
        .collect();
    if !names.is_empty() {
        let mut listed = names.iter().take(3).copied().collect::<Vec<_>>().join("、");
        if names.len() > 3 {
            listed.push_str(" 等");
        }
        return Err(format!(
            "凭证「{}」仍关联主机 {}，请先为这些主机更换凭证。",
            credential.name, listed
        ));
    }

    store.credentials.retain(|item| item.id != id);
    store_connections(dir, &store)?;
    let mut secrets = read_secrets(dir)?;
    secrets.remove(id);
    write_secrets(dir, &secrets)
}

fn save_host_at(dir: &Path, input: ManagedHostInput) -> Result<ManagedHost, String> {
    let input = input.with_default_port();
    let name = input.name.trim().to_string();
    let address = input.address.trim().to_string();
    if name.is_empty() {
        return Err("请填写主机名称".to_string());
    }
    if address.is_empty() {
        return Err("请填写主机地址".to_string());
    }
    if address.chars().any(char::is_whitespace) {
        return Err("主机地址不能包含空格".to_string());
    }
    if address.starts_with('-') {
        return Err("主机地址不能以 - 开头".to_string());
    }
    if input.port == 0 {
        return Err("SSH 端口需在 1 到 65535 之间".to_string());
    }

    let mut store = load_store(dir)?;
    if !store
        .credentials
        .iter()
        .any(|credential| credential.id == input.credential_id)
    {
        return Err("所选凭证不存在，请重新选择".to_string());
    }

    let requested_id = input.id.trim().to_string();
    let existing = if requested_id.is_empty() {
        None
    } else {
        store.hosts.iter().position(|item| item.id == requested_id)
    };
    if store.hosts.iter().any(|item| {
        Some(item.id.as_str()) != existing.map(|index| store.hosts[index].id.as_str())
            && same_name(&item.name, &name)
    }) {
        return Err(format!("已存在名为「{name}」的主机"));
    }

    let id = if requested_id.is_empty() {
        next_id("host", |candidate| {
            store.hosts.iter().any(|item| item.id == candidate)
        })
    } else {
        requested_id
    };

    let record = ManagedHost {
        id: id.clone(),
        name,
        address,
        port: input.port,
        credential_id: input.credential_id,
        deploy_dir: input.deploy_dir.trim().to_string(),
        notes: input.notes.trim().to_string(),
        updated_at: now_millis(),
    };
    match existing {
        Some(index) => store.hosts[index] = record.clone(),
        None => store.hosts.push(record.clone()),
    }
    if store.active_host_id.is_empty() {
        store.active_host_id = id;
    }
    store_connections(dir, &store)?;
    Ok(record)
}

fn delete_host_at(dir: &Path, id: &str) -> Result<ConnectionInventory, String> {
    let mut store = load_store(dir)?;
    let before = store.hosts.len();
    store.hosts.retain(|host| host.id != id);
    if store.hosts.len() == before {
        return Err("主机不存在或已被删除".to_string());
    }
    if store.active_host_id == id {
        store.active_host_id = store
            .hosts
            .first()
            .map(|host| host.id.clone())
            .unwrap_or_default();
    }
    store_connections(dir, &store)?;
    inventory_at(dir)
}

fn set_active_host_at(dir: &Path, id: &str) -> Result<ConnectionInventory, String> {
    let mut store = load_store(dir)?;
    if !store.hosts.iter().any(|host| host.id == id) {
        return Err("主机不存在或已被删除".to_string());
    }
    store.active_host_id = id.to_string();
    store_connections(dir, &store)?;
    inventory_at(dir)
}

fn host_connection_at(dir: &Path, id: &str) -> Result<(ManagedHost, SshCredential), String> {
    let store = load_store(dir)?;
    let host = store
        .hosts
        .iter()
        .find(|host| host.id == id)
        .cloned()
        .ok_or_else(|| "主机不存在或已被删除".to_string())?;
    let credential = store
        .credentials
        .iter()
        .find(|credential| credential.id == host.credential_id)
        .cloned()
        .ok_or_else(|| "主机关联的凭证不存在，请编辑主机后重新选择".to_string())?;
    Ok((host, credential))
}

#[tauri::command]
pub fn list_connection_inventory(app: AppHandle) -> Result<ConnectionInventory, String> {
    inventory_at(&config_dir(&app)?)
}

#[tauri::command]
pub fn save_ssh_credential(
    app: AppHandle,
    credential: SshCredentialInput,
) -> Result<SshCredential, String> {
    save_credential_at(&config_dir(&app)?, credential)
}

#[tauri::command]
pub fn delete_ssh_credential(app: AppHandle, id: String) -> Result<(), String> {
    delete_credential_at(&config_dir(&app)?, &id)
}

#[tauri::command]
pub fn save_managed_host(app: AppHandle, host: ManagedHostInput) -> Result<ManagedHost, String> {
    save_host_at(&config_dir(&app)?, host)
}

#[tauri::command]
pub fn delete_managed_host(app: AppHandle, id: String) -> Result<ConnectionInventory, String> {
    delete_host_at(&config_dir(&app)?, &id)
}

#[tauri::command]
pub fn set_active_managed_host(app: AppHandle, id: String) -> Result<ConnectionInventory, String> {
    set_active_host_at(&config_dir(&app)?, &id)
}

#[tauri::command]
pub fn test_managed_host_connection(app: AppHandle, id: String) -> Result<SshTestResult, String> {
    let (host, credential) = host_connection_at(&config_dir(&app)?, &id)?;
    test_ssh_connection(
        host.address,
        host.port,
        credential.username,
        if credential.private_key_path.is_empty() {
            None
        } else {
            Some(credential.private_key_path)
        },
    )
}

#[tauri::command]
pub fn setup_managed_host_access(app: AppHandle, id: String) -> Result<SshTestResult, String> {
    let dir = config_dir(&app)?;
    let (host, credential) = host_connection_at(&dir, &id)?;
    let password = read_secrets(&dir)?.remove(&credential.id);
    setup_remote_ssh_key(
        host.address,
        host.port,
        credential.username,
        password,
        if credential.private_key_path.is_empty() {
            None
        } else {
            Some(credential.private_key_path)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deploy-desktop-connections-{label}-{}-{}",
            std::process::id(),
            now_millis()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn credential(name: &str) -> SshCredentialInput {
        SshCredentialInput {
            name: name.to_string(),
            username: "root".to_string(),
            private_key_path: "/tmp/id_ed25519_deploy".to_string(),
            password: None,
            ..SshCredentialInput::default()
        }
    }

    fn host(name: &str, credential_id: &str) -> ManagedHostInput {
        ManagedHostInput {
            name: name.to_string(),
            address: "192.0.2.10".to_string(),
            port: 22,
            credential_id: credential_id.to_string(),
            deploy_dir: "/opt/deploy".to_string(),
            ..ManagedHostInput::default()
        }
    }

    #[test]
    fn password_is_stored_separately_and_not_returned() {
        let dir = temp_dir("secrets");
        let mut input = credential("生产环境");
        input.password = Some("secret-password".to_string());
        let saved = save_credential_at(&dir, input).unwrap();

        assert!(saved.has_password);
        let connections = fs::read_to_string(connections_path(&dir)).unwrap();
        assert!(!connections.contains("secret-password"));
        let secrets = read_secrets(&dir).unwrap();
        assert_eq!(
            secrets.get(&saved.id).map(String::as_str),
            Some("secret-password")
        );

        let inventory = inventory_at(&dir).unwrap();
        assert!(inventory.credentials[0].has_password);
        assert!(!serde_json::to_string(&inventory)
            .unwrap()
            .contains("secret-password"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(secrets_path(&dir))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn host_lifecycle_keeps_one_active_host() {
        let dir = temp_dir("hosts");
        let saved_credential = save_credential_at(&dir, credential("运维账号")).unwrap();
        let first = save_host_at(&dir, host("生产主机", &saved_credential.id)).unwrap();
        let second = save_host_at(&dir, host("备用主机", &saved_credential.id)).unwrap();

        let inventory = inventory_at(&dir).unwrap();
        assert_eq!(inventory.active_host_id, first.id);

        let inventory = set_active_host_at(&dir, &second.id).unwrap();
        assert_eq!(inventory.active_host_id, second.id);

        let inventory = delete_host_at(&dir, &second.id).unwrap();
        assert_eq!(inventory.active_host_id, first.id);

        let error = delete_credential_at(&dir, &saved_credential.id).unwrap_err();
        assert!(error.contains("仍关联主机"));

        delete_host_at(&dir, &first.id).unwrap();
        delete_credential_at(&dir, &saved_credential.id).unwrap();
        let inventory = inventory_at(&dir).unwrap();
        assert!(inventory.credentials.is_empty());
        assert!(inventory.hosts.is_empty());
        assert!(inventory.active_host_id.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_option_like_credentials_and_addresses() {
        let dir = temp_dir("validation");
        let mut unsafe_credential = credential("异常账号");
        unsafe_credential.username = "-oProxyCommand=whoami".to_string();
        let error = save_credential_at(&dir, unsafe_credential).unwrap_err();
        assert!(error.contains("登录用户名"));

        let saved_credential = save_credential_at(&dir, credential("运维账号")).unwrap();
        let mut unsafe_host = host("异常主机", &saved_credential.id);
        unsafe_host.address = "-oProxyCommand=whoami".to_string();
        let error = save_host_at(&dir, unsafe_host).unwrap_err();
        assert!(error.contains("主机地址"));

        fs::remove_dir_all(dir).unwrap();
    }
}
