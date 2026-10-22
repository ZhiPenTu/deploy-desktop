use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tauri::{ipc::Channel, AppHandle, Manager};

const FLOW_API: &str = "https://openapi-rdc.aliyuncs.com/oapi/v1/flow/organizations";
const FLOW_WEB: &str = "https://flow.aliyun.com/pipelines";
const SECRET_SERVICE: &str = "deploy-desktop-artifact";
const MAX_ARTIFACT_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const FLOW_RUN_PAGE_SIZE: u32 = 20;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactSource {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub org_id: String,
    pub pipeline_id: String,
    pub artifact_name: String,
    pub version: String,
    pub download_url: String,
    pub token_id: String,
    pub has_token: bool,
    pub last_version: String,
    pub last_bundle_dir: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactSourceInput {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub org_id: String,
    pub pipeline_id: String,
    pub artifact_name: String,
    pub version: String,
    pub download_url: String,
    pub token_id: String,
}

/// 可复用的云效个人访问令牌，只保存名称与时间，令牌内容单独存放。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactToken {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactTokenInput {
    pub id: String,
    pub name: String,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactTokenView {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub has_token: bool,
    pub in_use: usize,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TokenLibraryFile {
    tokens: Vec<ArtifactToken>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactRelease {
    pub run_id: i64,
    pub run_url: String,
    pub run_status: String,
    pub version: String,
    pub commit: String,
    pub commit_short: String,
    pub commit_title: String,
    pub branch: String,
    pub built_at: i64,
    pub artifact_name: String,
    pub repo_id: String,
    pub size_bytes: u64,
    pub md5: String,
    pub download_url: String,
    pub local_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReleasePage {
    pub items: Vec<ArtifactRelease>,
    pub page: u32,
    pub per_page: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub stage: String,
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactBundle {
    pub dir: String,
    pub version: String,
    pub commit: String,
    pub md5: String,
    pub size_bytes: u64,
    pub files: usize,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SourceFile {
    sources: Vec<ArtifactSource>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct CachedReleaseEntry {
    signature: String,
    cached_at: i64,
    has_more: bool,
    items: Vec<ArtifactRelease>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct ReleaseCacheFile {
    sources: std::collections::BTreeMap<String, CachedReleaseEntry>,
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("读取应用配置目录失败：{e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建应用配置目录失败：{e}"))?;
    Ok(dir)
}

fn sources_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join("artifact-sources.json"))
}

fn load_sources(app: &AppHandle) -> Result<Vec<ArtifactSource>, String> {
    let path = sources_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读取制品源配置失败：{e}"))?;
    let file: SourceFile =
        serde_json::from_str(&content).map_err(|e| format!("解析制品源配置失败：{e}"))?;
    Ok(file.sources)
}

fn store_sources(app: &AppHandle, sources: &[ArtifactSource]) -> Result<(), String> {
    let path = sources_path(app)?;
    let content = serde_json::to_string_pretty(&SourceFile {
        sources: sources.to_vec(),
    })
        .map_err(|e| format!("序列化制品源配置失败：{e}"))?;
    fs::write(&path, content).map_err(|e| format!("保存制品源配置失败：{e}"))
}

fn secrets_file(dir: &Path) -> PathBuf {
    dir.join("artifact-tokens.json")
}

fn token_library_path(dir: &Path) -> PathBuf {
    dir.join("artifact-token-library.json")
}

fn read_token_library(dir: &Path) -> Result<Vec<ArtifactToken>, String> {
    let path = token_library_path(dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取访问令牌列表失败：{e}"))?;
    let file: TokenLibraryFile =
        serde_json::from_str(&content).map_err(|e| format!("解析访问令牌列表失败：{e}"))?;
    Ok(file.tokens)
}

fn write_token_library(dir: &Path, tokens: &[ArtifactToken]) -> Result<(), String> {
    let path = token_library_path(dir);
    let content = serde_json::to_string_pretty(&TokenLibraryFile {
        tokens: tokens.to_vec(),
    })
    .map_err(|e| format!("序列化访问令牌列表失败：{e}"))?;
    fs::write(&path, content).map_err(|e| format!("保存访问令牌列表失败：{e}"))
}

/// 令牌内容在凭据文件中使用独立前缀，与按制品源保存的历史记录区分开。
fn token_secret_key(id: &str) -> String {
    format!("token:{id}")
}

fn read_secrets(dir: &Path) -> std::collections::BTreeMap<String, String> {
    let path = secrets_file(dir);
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn write_secrets(
    dir: &Path,
    secrets: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let path = secrets_file(dir);
    let content = serde_json::to_string_pretty(secrets).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("写入本地凭据文件失败：{e}"))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("写入本地凭据文件失败：{e}"))?;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        fs::write(&path, content).map_err(|e| format!("写入本地凭据文件失败：{e}"))?;
    }
    Ok(())
}

fn keychain_marker(id: &str) -> String {
    format!("__keychain_migrated:{id}")
}

/// 旧版本把令牌放在系统钥匙串里。macOS 上应用未签名时每次读取都会弹授权框，
/// 因此这里只尝试迁移一次，之后统一使用本机凭据文件。
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn legacy_keychain_token(id: &str) -> Option<String> {
    keyring::Entry::new(SECRET_SERVICE, id)
        .ok()?
        .get_password()
        .ok()
        .filter(|value| !value.is_empty())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn delete_legacy_keychain_token(id: &str) {
    if let Ok(entry) = keyring::Entry::new(SECRET_SERVICE, id) {
        let _ = entry.delete_credential();
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn legacy_keychain_token(_id: &str) -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn delete_legacy_keychain_token(_id: &str) {}

fn secret_set(dir: &Path, id: &str, token: &str) -> Result<(), String> {
    let mut secrets = read_secrets(dir);
    secrets.insert(id.to_string(), token.to_string());
    secrets.remove(&keychain_marker(id));
    write_secrets(dir, &secrets)
}

fn secret_get(dir: &Path, id: &str) -> Result<Option<String>, String> {
    let secrets = read_secrets(dir);
    if let Some(token) = secrets.get(id) {
        return Ok(Some(token.clone()));
    }
    if secrets.contains_key(&keychain_marker(id)) {
        return Ok(None);
    }

    let legacy = legacy_keychain_token(id);
    let mut secrets = read_secrets(dir);
    secrets.insert(keychain_marker(id), "1".to_string());
    if let Some(token) = legacy {
        secrets.insert(id.to_string(), token.clone());
        write_secrets(dir, &secrets)?;
        delete_legacy_keychain_token(id);
        return Ok(Some(token));
    }
    write_secrets(dir, &secrets)?;
    Ok(None)
}

fn secret_delete(dir: &Path, id: &str) -> Result<(), String> {
    let mut secrets = read_secrets(dir);
    secrets.remove(id);
    secrets.remove(&keychain_marker(id));
    delete_legacy_keychain_token(id);
    write_secrets(dir, &secrets)
}

fn stored_token(dir: &Path, token_id: &str) -> Result<Option<String>, String> {
    Ok(secret_get(dir, &token_secret_key(token_id))?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

/// 制品源绑定令牌库时只读取该令牌；未绑定时沿用按制品源保存的历史令牌。
fn source_token(dir: &Path, source: &ArtifactSource) -> Result<Option<String>, String> {
    let token_id = source.token_id.trim();
    if !token_id.is_empty() {
        return stored_token(dir, token_id);
    }
    Ok(secret_get(dir, &source.id)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

fn new_token_id(tokens: &[ArtifactToken]) -> String {
    let mut stamp = now_millis();
    loop {
        let id = format!("tok-{stamp:x}");
        if !tokens.iter().any(|token| token.id == id) {
            return id;
        }
        stamp += 1;
    }
}

fn unique_token_name(tokens: &[ArtifactToken], base: &str) -> String {
    if !tokens.iter().any(|token| token.name == base) {
        return base.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base} {index}");
        if !tokens.iter().any(|token| token.name == candidate) {
            return candidate;
        }
        index += 1;
    }
}

/// 旧版本按制品源保存令牌。首次打开令牌列表时统一收进令牌库，
/// 相同内容的令牌只保留一条，制品源改绑到该令牌。
/// 设置页会同时读取令牌与制品源，迁移需要串行执行，避免重复写入。
static LEGACY_TOKEN_MIGRATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn migrate_legacy_tokens(app: &AppHandle) -> Result<(), String> {
    let _guard = LEGACY_TOKEN_MIGRATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = config_dir(app)?;
    let mut sources = load_sources(app)?;
    if sources.iter().all(|source| !source.token_id.trim().is_empty()) {
        return Ok(());
    }
    let mut tokens = read_token_library(&dir)?;
    let mut changed = false;

    for source in sources.iter_mut() {
        if !source.token_id.trim().is_empty() {
            continue;
        }
        let value = match secret_get(&dir, &source.id)? {
            Some(value) => value.trim().to_string(),
            None => continue,
        };
        if value.is_empty() {
            continue;
        }

        let mut matched = None;
        for token in tokens.iter() {
            if stored_token(&dir, &token.id)?.as_deref() == Some(value.as_str()) {
                matched = Some(token.id.clone());
                break;
            }
        }

        let token_id = match matched {
            Some(id) => id,
            None => {
                let base = if source.name.trim().is_empty() {
                    "云效访问令牌".to_string()
                } else {
                    format!("{}的令牌", source.name.trim())
                };
                let now = now_millis() as u64;
                let id = new_token_id(&tokens);
                secret_set(&dir, &token_secret_key(&id), &value)?;
                tokens.push(ArtifactToken {
                    id: id.clone(),
                    name: unique_token_name(&tokens, &base),
                    created_at: now,
                    updated_at: now,
                });
                id
            }
        };

        source.token_id = token_id;
        source.has_token = true;
        secret_delete(&dir, &source.id)?;
        changed = true;
    }

    if changed {
        write_token_library(&dir, &tokens)?;
        store_sources(app, &sources)?;
    }
    Ok(())
}

fn token_view(
    dir: &Path,
    token: &ArtifactToken,
    sources: &[ArtifactSource],
) -> Result<ArtifactTokenView, String> {
    Ok(ArtifactTokenView {
        id: token.id.clone(),
        name: token.name.clone(),
        created_at: token.created_at,
        updated_at: token.updated_at,
        has_token: stored_token(dir, &token.id)?.is_some(),
        in_use: sources
            .iter()
            .filter(|source| source.token_id.trim() == token.id)
            .count(),
    })
}

#[tauri::command]
pub fn list_artifact_sources(app: AppHandle) -> Result<Vec<ArtifactSource>, String> {
    let _ = migrate_legacy_tokens(&app);
    let dir = config_dir(&app)?;
    let mut sources = load_sources(&app)?;
    for source in sources.iter_mut() {
        source.has_token = source_token(&dir, source)?.is_some();
    }
    Ok(sources)
}

#[tauri::command]
pub fn list_artifact_tokens(app: AppHandle) -> Result<Vec<ArtifactTokenView>, String> {
    let _ = migrate_legacy_tokens(&app);
    let dir = config_dir(&app)?;
    let tokens = read_token_library(&dir)?;
    let sources = load_sources(&app)?;
    tokens
        .iter()
        .map(|token| token_view(&dir, token, &sources))
        .collect()
}

#[tauri::command]
pub fn save_artifact_token(
    app: AppHandle,
    token: ArtifactTokenInput,
) -> Result<ArtifactTokenView, String> {
    let dir = config_dir(&app)?;
    let name = token.name.trim().to_string();
    if name.is_empty() {
        return Err("请填写令牌名称".to_string());
    }

    let mut tokens = read_token_library(&dir)?;
    let requested = token.id.trim().to_string();
    let existing = if requested.is_empty() {
        None
    } else {
        match tokens.iter().position(|item| item.id == requested) {
            Some(index) => Some(index),
            None => return Err("该令牌已不存在，请重新选择".to_string()),
        }
    };
    if tokens
        .iter()
        .enumerate()
        .any(|(index, item)| Some(index) != existing && item.name == name)
    {
        return Err("已有同名令牌，请换一个名称".to_string());
    }

    let value = token
        .token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if existing.is_none() && value.is_none() {
        return Err("请填写云效个人访问令牌".to_string());
    }

    let id = match existing {
        Some(index) => tokens[index].id.clone(),
        None => new_token_id(&tokens),
    };
    if let Some(value) = value {
        secret_set(&dir, &token_secret_key(&id), value)?;
    }

    let now = now_millis() as u64;
    let record = ArtifactToken {
        created_at: existing.map(|index| tokens[index].created_at).unwrap_or(now),
        id,
        name,
        updated_at: now,
    };
    match existing {
        Some(index) => tokens[index] = record.clone(),
        None => tokens.push(record.clone()),
    }
    write_token_library(&dir, &tokens)?;

    let sources = load_sources(&app)?;
    token_view(&dir, &record, &sources)
}

#[tauri::command]
pub fn delete_artifact_token(app: AppHandle, id: String) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("请选择要删除的令牌".to_string());
    }

    let sources = load_sources(&app)?;
    let in_use = sources
        .iter()
        .filter(|source| source.token_id.trim() == id)
        .count();
    if in_use > 0 {
        return Err(format!(
            "该令牌仍被 {in_use} 个制品源使用，请先为这些制品源更换令牌"
        ));
    }

    let mut tokens = read_token_library(&dir)?;
    let before = tokens.len();
    tokens.retain(|item| item.id != id);
    if tokens.len() == before {
        return Err("该令牌已不存在".to_string());
    }
    write_token_library(&dir, &tokens)?;
    secret_delete(&dir, &token_secret_key(&id))?;
    Ok(())
}

#[tauri::command]
pub fn save_artifact_source(
    app: AppHandle,
    source: ArtifactSourceInput,
) -> Result<ArtifactSource, String> {
    let dir = config_dir(&app)?;
    let name = source.name.trim().to_string();
    if name.is_empty() {
        return Err("请填写制品源名称".to_string());
    }
    let kind = if source.kind.trim().is_empty() {
        "yunxiaoFlow".to_string()
    } else {
        source.kind.trim().to_string()
    };
    if kind == "yunxiaoFlow" {
        if source.org_id.trim().is_empty() || source.pipeline_id.trim().is_empty() {
            return Err("云效流水线制品源需要填写组织 ID 与流水线 ID".to_string());
        }
    } else if kind == "url" && source.download_url.trim().is_empty() {
        return Err("直接下载源需要填写下载地址".to_string());
    } else if kind != "yunxiaoFlow" && kind != "url" {
        return Err(format!("不支持的制品源类型：{kind}"));
    }
    if kind == "url" {
        validate_artifact_version(&source.version)?;
    }

    let id = if source.id.trim().is_empty() {
        format!("src-{:x}", now_millis())
    } else {
        source.id.trim().to_string()
    };

    let mut sources = load_sources(&app)?;
    let existing = sources.iter().position(|item| item.id == id);
    let last_version = existing.map(|idx| sources[idx].last_version.clone()).unwrap_or_default();
    let last_bundle_dir = existing
        .map(|idx| sources[idx].last_bundle_dir.clone())
        .unwrap_or_default();

    let token_id = if kind == "yunxiaoFlow" {
        source.token_id.trim().to_string()
    } else {
        String::new()
    };
    if !token_id.is_empty() {
        let known = read_token_library(&dir)?
            .iter()
            .any(|token| token.id == token_id);
        if !known {
            return Err("所选访问令牌已不存在，请重新选择".to_string());
        }
        // 绑定令牌库后不再保留按制品源保存的旧令牌。
        secret_delete(&dir, &id)?;
    }

    let mut record = ArtifactSource {
        id: id.clone(),
        name,
        version: if kind == "url" { source.version.trim().to_string() } else { String::new() },
        kind,
        org_id: source.org_id.trim().to_string(),
        pipeline_id: source.pipeline_id.trim().to_string(),
        artifact_name: if source.artifact_name.trim().is_empty() {
            "deploy-bundle".to_string()
        } else {
            source.artifact_name.trim().to_string()
        },
        download_url: source.download_url.trim().to_string(),
        token_id,
        has_token: false,
        last_version,
        last_bundle_dir,
    };
    record.has_token = source_token(&dir, &record)?.is_some();

    match existing {
        Some(idx) => sources[idx] = record.clone(),
        None => sources.push(record.clone()),
    }
    store_sources(&app, &sources)?;

    Ok(record)
}

#[tauri::command]
pub fn delete_artifact_source(app: AppHandle, id: String) -> Result<(), String> {
    let dir = config_dir(&app)?;
    let mut sources = load_sources(&app)?;
    sources.retain(|item| item.id != id);
    store_sources(&app, &sources)?;
    secret_delete(&dir, &id)?;
    remove_release_cache(&dir, &id);
    Ok(())
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

fn http_get_json(token: &str, url: &str) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let response = client
        .get(url)
        .header("x-yunxiao-token", token)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| {
            log::warn!(target: "artifact", "读取云效制品数据失败：{e}");
            if e.is_timeout() {
                "读取云效响应超时，请检查网络后重试".to_string()
            } else if e.is_connect() {
                "无法连接云效，请检查网络后重试".to_string()
            } else {
                "读取云效数据失败，请检查网络后重试".to_string()
            }
        })?;
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err("云效令牌无效或权限不足，请重新填写个人访问令牌".to_string());
    }
    if !status.is_success() {
        return Err(format!("云效接口返回 HTTP {status}"));
    }
    let body = response
        .text()
        .map_err(|e| format!("读取云效接口响应失败：{e}"))?;
    serde_json::from_str::<Value>(&body).map_err(|e| format!("解析云效接口响应失败：{e}"))
}

fn collect_artifacts(value: &Value, out: &mut Vec<Value>) {
    match value {
        Value::Object(map) => {
            if map.contains_key("downloadUrl") && map.contains_key("md5") {
                out.push(value.clone());
            }
            for item in map.values() {
                collect_artifacts(item, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_artifacts(item, out);
            }
        }
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    collect_artifacts(&parsed, out);
                }
            }
        }
        _ => {}
    }
}

fn key_matches(candidate: &str, key: &str, dotted: &str) -> bool {
    candidate == key || candidate.ends_with(dotted)
}

fn find_key_string(value: &Value, key: &str) -> Option<String> {
    let dotted = format!(".{key}");
    find_key_string_inner(value, key, &dotted)
}

fn find_key_string_inner(value: &Value, key: &str, dotted: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(found)) = map.get(key) {
                return Some(found.clone());
            }
            for (name, item) in map {
                if key_matches(name, key, dotted) {
                    if let Value::String(found) = item {
                        return Some(found.clone());
                    }
                }
            }
            for item in map.values() {
                if let Some(found) = find_key_string_inner(item, key, dotted) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items
            .iter()
            .find_map(|item| find_key_string_inner(item, key, dotted)),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    return find_key_string_inner(&parsed, key, dotted);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_key_i64(value: &Value, key: &str) -> i64 {
    let dotted = format!(".{key}");
    find_key_i64_inner(value, key, &dotted)
}

fn find_key_i64_inner(value: &Value, key: &str, dotted: &str) -> i64 {
    match value {
        Value::Object(map) => {
            for (name, item) in map {
                if key_matches(name, key, dotted) {
                    if let Some(number) = item.as_i64() {
                        return number;
                    }
                }
            }
            for item in map.values() {
                let found = find_key_i64_inner(item, key, dotted);
                if found > 0 {
                    return found;
                }
            }
            0
        }
        Value::Array(items) => items
            .iter()
            .map(|item| find_key_i64_inner(item, key, dotted))
            .find(|value| *value > 0)
            .unwrap_or_default(),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    return find_key_i64_inner(&parsed, key, dotted);
                }
            }
            0
        }
        _ => 0,
    }
}

fn pick_artifact(value: &Value, artifact_name: &str) -> Option<Value> {
    let mut candidates = Vec::new();
    collect_artifacts(value, &mut candidates);
    candidates
        .iter()
        .find(|item| {
            item.get("artifact")
                .and_then(Value::as_str)
                .map(|name| {
                    name == artifact_name
                        || name == format!("{artifact_name}.tgz")
                        || name == format!("{artifact_name}.tar.gz")
                })
                .unwrap_or(false)
        })
        .cloned()
}

fn json_i64(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}

fn json_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn json_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn safe_version_key(version: &str) -> String {
    version
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn validate_artifact_version(version: &str) -> Result<&str, String> {
    let version = version.trim();
    if version.is_empty() || version == "manual" {
        return Err("制品缺少版本号，请补充版本后重试".to_string());
    }
    if version.len() > 128
        || !version.starts_with(|character: char| character.is_ascii_alphanumeric())
        || !version.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        return Err("制品版本号格式无效，请使用字母、数字、点、短横线或下划线，以字母或数字开头，最多 128 个字符".to_string());
    }
    Ok(version)
}

fn url_release(source: &ArtifactSource) -> Result<ArtifactRelease, String> {
    let version = if source.version.trim().is_empty() {
        &source.last_version
    } else {
        &source.version
    };
    let version = validate_artifact_version(version)
        .map_err(|_| "请编辑制品源，填写与交付包一致的版本号".to_string())?;
    Ok(ArtifactRelease {
        run_id: 0,
        run_url: String::new(),
        run_status: "manual".to_string(),
        version: version.to_string(),
        artifact_name: source.artifact_name.clone(),
        download_url: source.download_url.clone(),
        ..Default::default()
    })
}

fn release_from_flow_detail(
    source: &ArtifactSource,
    run_id: i64,
    detail: &Value,
) -> Result<Option<ArtifactRelease>, String> {
    let Some(artifact) = pick_artifact(detail, &source.artifact_name) else {
        return Ok(None);
    };
    let download_url = json_str(&artifact, "downloadUrl").to_string();
    if download_url.is_empty() {
        return Ok(None);
    }
    let version = artifact.get("version").and_then(|value| {
        value.as_str().map(str::to_string).or_else(|| value.as_u64().map(|number| number.to_string()))
    }).unwrap_or_default();
    let version = validate_artifact_version(&version)
        .map_err(|error| format!("运行 #{run_id}：{error}，请重新构建并上传带版本号的制品"))?;

    Ok(Some(ArtifactRelease {
        run_id,
        run_url: format!("{FLOW_WEB}/{}/runs/{}", source.pipeline_id, run_id),
        run_status: json_str(detail, "status").to_string(),
        version: version.to_string(),
        commit: find_key_string(detail, "CI_COMMIT_SHA").unwrap_or_default(),
        commit_short: find_key_string(detail, "CI_COMMIT_ID").unwrap_or_default(),
        commit_title: find_key_string(detail, "CI_COMMIT_TITLE").unwrap_or_default(),
        branch: find_key_string(detail, "CI_COMMIT_REF_NAME").unwrap_or_default(),
        built_at: find_key_i64(detail, "startTime"),
        artifact_name: json_str(&artifact, "artifact").to_string(),
        repo_id: json_str(&artifact, "repoId").to_string(),
        size_bytes: json_u64(&artifact, "fileSize")
            .max(json_u64(&artifact, "size"))
            .max(find_key_i64(detail, "fileSize").max(0) as u64),
        md5: json_str(&artifact, "md5").to_string(),
        download_url,
        local_dir: String::new(),
    }))
}

fn cached_bundle_dir(
    config_root: &Path,
    source: &ArtifactSource,
    release: &ArtifactRelease,
) -> Option<PathBuf> {
    let version = safe_version_key(&release.version);
    let expected = config_root.join("bundles").join(&source.id).join(version);
    if bundle_has_version(&expected, &release.version) {
        return Some(expected);
    }

    let last_dir = source.last_bundle_dir.trim();
    if last_dir.is_empty()
        || source.last_version.trim() != release.version.trim()
    {
        return None;
    }
    let last_dir = PathBuf::from(last_dir);
    bundle_has_version(&last_dir, &release.version).then_some(last_dir)
}

fn bundle_has_version(dir: &Path, version: &str) -> bool {
    fs::read_to_string(dir.join("bundle.json"))
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .is_some_and(|meta| {
            meta.get("metadata")
                .map(|metadata| json_str(metadata, "version").trim() == version.trim())
                .unwrap_or(false)
        })
}

fn release_cache_path(config_root: &Path) -> PathBuf {
    config_root.join("artifact-releases-cache.json")
}

fn release_cache(config_root: &Path) -> ReleaseCacheFile {
    fs::read_to_string(release_cache_path(config_root))
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn source_signature(source: &ArtifactSource) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        source.kind, source.org_id, source.pipeline_id, source.artifact_name, source.download_url
    )
}

fn release_cache_key(item: &ArtifactRelease) -> String {
    format!("{}|{}|{}", item.run_id, item.artifact_name, item.version)
}

/// 记录最近一次成功读取的云端版本列表，供下次打开页面或断网时先展示。
/// 下载地址可能带有临时签名，只留在当前会话，不写入本地缓存文件。
fn store_release_cache(config_root: &Path, source: &ArtifactSource, page: &ArtifactReleasePage) {
    let mut file = release_cache(config_root);
    let signature = source_signature(source);
    let entry = file.sources.entry(source.id.clone()).or_default();
    if entry.signature != signature {
        entry.items.clear();
        entry.has_more = false;
    }
    let incoming = page.items.iter().cloned().map(|mut item| {
        item.download_url.clear();
        item
    });
    if page.page <= 1 {
        entry.items = incoming.collect();
    } else {
        for item in incoming {
            if !entry.items.iter().any(|known| release_cache_key(known) == release_cache_key(&item)) {
                entry.items.push(item);
            }
        }
    }
    entry.signature = signature;
    entry.has_more = page.has_more;
    entry.cached_at = now_millis().min(i64::MAX as u128) as i64;
    if let Ok(content) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(release_cache_path(config_root), content);
    }
}

fn remove_release_cache(config_root: &Path, id: &str) {
    let mut file = release_cache(config_root);
    if file.sources.remove(id).is_none() {
        return;
    }
    if let Ok(content) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(release_cache_path(config_root), content);
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let yoe = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = yoe * 365 + yoe / 4 - yoe / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

fn parse_rfc3339_millis(text: &str) -> Option<i64> {
    let text = text.trim();
    let year: i64 = text.get(0..4)?.parse().ok()?;
    let month: i64 = text.get(5..7)?.parse().ok()?;
    let day: i64 = text.get(8..10)?.parse().ok()?;
    let hour: i64 = text.get(11..13)?.parse().ok()?;
    let minute: i64 = text.get(14..16)?.parse().ok()?;
    let second: i64 = text.get(17..19)?.parse().ok()?;
    if text.as_bytes().get(4) != Some(&b'-') || text.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let mut offset_seconds = 0i64;
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        rest = fraction.trim_start_matches(|character: char| character.is_ascii_digit());
    }
    if rest.starts_with('+') || rest.starts_with('-') {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let digits = rest.get(1..)?;
        let (offset_hour, offset_minute) = match digits.split_once(':') {
            Some((hour, minute)) => (hour.parse::<i64>().ok()?, minute.parse::<i64>().ok()?),
            None => (
                digits.get(0..2)?.parse::<i64>().ok()?,
                digits.get(2..4)?.parse::<i64>().ok()?,
            ),
        };
        offset_seconds = sign * (offset_hour * 3600 + offset_minute * 60);
    }
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_seconds;
    Some(seconds * 1000)
}

fn bundle_meta_size(meta: &Value) -> u64 {
    meta.get("files")
        .and_then(Value::as_array)
        .map(|files| files.iter().map(|item| json_u64(item, "size")).sum())
        .unwrap_or_default()
}

fn release_from_local_bundle(source: &ArtifactSource, dir: &Path) -> Option<ArtifactRelease> {
    let meta: Value = fs::read_to_string(dir.join("bundle.json"))
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())?;
    // 只识别当前格式的交付包；旧格式目录不再展示，也无法直接使用。
    let metadata = meta.get("metadata")?;
    let version = validate_artifact_version(json_str(metadata, "version"))
        .ok()?
        .to_string();
    Some(ArtifactRelease {
        run_status: "cached".to_string(),
        version,
        commit: json_str(metadata, "commit").to_string(),
        commit_short: json_str(metadata, "commitShort").to_string(),
        branch: json_str(metadata, "branch").to_string(),
        built_at: parse_rfc3339_millis(json_str(metadata, "builtAt")).unwrap_or_default(),
        artifact_name: source.artifact_name.clone(),
        size_bytes: bundle_meta_size(&meta),
        local_dir: dir.to_string_lossy().to_string(),
        ..Default::default()
    })
}

fn local_bundle_dirs(config_root: &Path, source: &ArtifactSource) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = fs::read_dir(config_root.join("bundles").join(&source.id)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    let last_dir = source.last_bundle_dir.trim();
    if !last_dir.is_empty() {
        let path = PathBuf::from(last_dir);
        if path.is_dir() && !dirs.iter().any(|item| item == &path) {
            dirs.push(path);
        }
    }
    dirs
}

fn cached_release_page(config_root: &Path, source: &ArtifactSource) -> ArtifactReleasePage {
    if source.kind == "url" {
        if let Ok(mut release) = url_release(source) {
            attach_local_bundle(config_root, source, &mut release);
            return ArtifactReleasePage {
                items: vec![release],
                page: 1,
                per_page: 1,
                has_more: false,
            };
        }
    }

    let file = release_cache(config_root);
    let mut items = Vec::new();
    let mut has_more = false;
    if let Some(entry) = file.sources.get(&source.id) {
        if entry.signature == source_signature(source) {
            items = entry.items.clone();
            has_more = entry.has_more;
        }
    }

    for item in &mut items {
        let local_dir = item.local_dir.trim();
        if !local_dir.is_empty() && !bundle_has_version(Path::new(local_dir), &item.version) {
            item.local_dir.clear();
        }
        if item.local_dir.is_empty() {
            if let Some(dir) = cached_bundle_dir(config_root, source, item) {
                item.local_dir = dir.to_string_lossy().to_string();
            }
        }
    }

    for local in local_bundle_dirs(config_root, source)
        .iter()
        .filter_map(|dir| release_from_local_bundle(source, dir))
    {
        match items.iter_mut().find(|item| item.version == local.version) {
            Some(existing) => {
                if existing.local_dir.is_empty() {
                    existing.local_dir = local.local_dir;
                }
                if existing.commit.is_empty() {
                    existing.commit = local.commit;
                }
                if existing.commit_short.is_empty() {
                    existing.commit_short = local.commit_short;
                }
                if existing.branch.is_empty() {
                    existing.branch = local.branch;
                }
                if existing.built_at == 0 {
                    existing.built_at = local.built_at;
                }
                if existing.size_bytes == 0 {
                    existing.size_bytes = local.size_bytes;
                }
            }
            None => items.push(local),
        }
    }

    items.sort_by(|left, right| {
        right
            .built_at
            .cmp(&left.built_at)
            .then_with(|| right.run_id.cmp(&left.run_id))
    });

    ArtifactReleasePage {
        items,
        page: 1,
        per_page: FLOW_RUN_PAGE_SIZE,
        has_more,
    }
}

fn attach_local_bundle(
    config_root: &Path,
    source: &ArtifactSource,
    release: &mut ArtifactRelease,
) {
    if let Some(dir) = cached_bundle_dir(config_root, source, release) {
        release.local_dir = dir.to_string_lossy().to_string();
    }
}

fn flow_run_items(
    token: &str,
    source: &ArtifactSource,
    page: u32,
) -> Result<Vec<Value>, String> {
    let url = format!(
        "{FLOW_API}/{}/pipelines/{}/runs?page={}&perPage={}",
        source.org_id, source.pipeline_id, page.max(1), FLOW_RUN_PAGE_SIZE
    );
    let runs = http_get_json(token, &url)?;
    runs.as_array()
        .cloned()
        .ok_or_else(|| "云效运行列表格式异常".to_string())
}

fn load_flow_release_page(
    config_root: &Path,
    source: &ArtifactSource,
    token: &str,
    page: u32,
) -> Result<ArtifactReleasePage, String> {
    let runs = flow_run_items(token, source, page)?;
    let mut releases = Vec::new();
    for run in runs
        .iter()
        .filter(|item| json_str(item, "status") == "SUCCESS")
    {
        let run_id = json_i64(run, "pipelineRunId");
        if run_id <= 0 {
            continue;
        }
        let detail_url = format!(
            "{FLOW_API}/{}/pipelines/{}/runs/{}",
            source.org_id, source.pipeline_id, run_id
        );
        let detail = http_get_json(token, &detail_url)?;
        if let Some(mut release) = release_from_flow_detail(source, run_id, &detail)? {
            attach_local_bundle(config_root, source, &mut release);
            releases.push(release);
        }
    }

    releases.sort_by(|left, right| {
        right
            .built_at
            .cmp(&left.built_at)
            .then_with(|| right.run_id.cmp(&left.run_id))
    });

    let page = ArtifactReleasePage {
        items: releases,
        page: page.max(1),
        per_page: FLOW_RUN_PAGE_SIZE,
        has_more: runs.len() >= FLOW_RUN_PAGE_SIZE as usize,
    };
    store_release_cache(config_root, source, &page);
    Ok(page)
}

fn load_artifact_release_page(
    config_root: &Path,
    source: &ArtifactSource,
    page: u32,
) -> Result<ArtifactReleasePage, String> {
    if source.kind == "url" {
        let mut release = url_release(source)?;
        attach_local_bundle(config_root, source, &mut release);
        return Ok(ArtifactReleasePage {
            items: vec![release],
            page: 1,
            per_page: 1,
            has_more: false,
        });
    }

    let token = source_token(config_root, source)?
        .ok_or_else(|| "尚未配置云效个人访问令牌，请在设置中选择令牌".to_string())?;
    load_flow_release_page(config_root, source, &token, page.max(1))
}

fn list_artifact_releases_blocking(
    app: AppHandle,
    id: String,
    page: Option<u32>,
) -> Result<ArtifactReleasePage, String> {
    let config_root = config_dir(&app)?;
    let sources = load_sources(&app)?;
    let source = sources
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "制品源不存在".to_string())?;
    load_artifact_release_page(&config_root, source, page.unwrap_or(1))
}

#[tauri::command]
pub async fn list_artifact_releases(
    app: AppHandle,
    id: String,
    page: Option<u32>,
) -> Result<ArtifactReleasePage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        list_artifact_releases_blocking(app, id, page)
    })
    .await
    .map_err(|error| format!("读取制品版本失败：{error}"))?
}

/// 读取上次成功获取的版本列表以及本机已解包的交付包，用于先渲染页面。
#[tauri::command]
pub async fn list_cached_artifact_releases(
    app: AppHandle,
    id: String,
) -> Result<ArtifactReleasePage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let id = id.trim().to_string();
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            return Err("非法的制品源标识".to_string());
        }
        let config_root = config_dir(&app)?;
        let sources = load_sources(&app)?;
        let source = sources
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| "制品源不存在".to_string())?;
        Ok(cached_release_page(&config_root, source))
    })
    .await
    .map_err(|error| format!("读取本地制品缓存失败：{error}"))?
}

#[tauri::command]
pub async fn fetch_artifact_release(
    app: AppHandle,
    id: String,
    run_id: Option<i64>,
) -> Result<ArtifactRelease, String> {
    crate::commands::blocking::run(move || fetch_artifact_release_blocking(app, id, run_id)).await
}

fn fetch_artifact_release_blocking(
    app: AppHandle,
    id: String,
    run_id: Option<i64>,
) -> Result<ArtifactRelease, String> {
    let config_root = config_dir(&app)?;
    let sources = load_sources(&app)?;
    let source = sources
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "制品源不存在".to_string())?;

    if source.kind == "url" {
        let mut release = url_release(source)?;
        attach_local_bundle(&config_root, source, &mut release);
        return Ok(release);
    }

    let token = source_token(&config_root, source)?
        .ok_or_else(|| "尚未配置云效个人访问令牌，请在设置中选择令牌".to_string())?;

    if let Some(run_id) = run_id.filter(|value| *value > 0) {
        let detail_url = format!(
            "{FLOW_API}/{}/pipelines/{}/runs/{}",
            source.org_id, source.pipeline_id, run_id
        );
        let detail = http_get_json(&token, &detail_url)?;
        let mut release = release_from_flow_detail(source, run_id, &detail)?
            .ok_or_else(|| "未在该运行中找到构建物上传结果".to_string())?;
        attach_local_bundle(&config_root, source, &mut release);
        return Ok(release);
    }

    load_flow_release_page(&config_root, source, &token, 1)?
        .items
        .into_iter()
        .next()
        .ok_or_else(|| "没有找到包含构建物的成功运行".to_string())
}

fn validate_download_url(url: &str, source: &ArtifactSource) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "下载地址格式无效".to_string())?;
    if parsed.scheme() != "https" {
        return Err("仅允许通过 HTTPS 下载交付包".to_string());
    }
    let host = parsed.host_str().unwrap_or_default();

    if source.kind == "url" {
        let expected = reqwest::Url::parse(source.download_url.trim())
            .map_err(|_| "制品源下载地址格式无效".to_string())?;
        if parsed.as_str() != expected.as_str() {
            return Err("下载地址与制品源配置不一致，已拒绝请求".to_string());
        }
        return Ok(());
    }

    let allowed = host == "aliyuncs.com"
        || host.ends_with(".aliyuncs.com")
        || host == "aliyun.com"
        || host.ends_with(".aliyun.com");
    if !allowed {
        return Err(format!("构建物下载域名不在允许范围内：{host}"));
    }
    Ok(())
}

fn download_to_file(
    url: &str,
    target: &Path,
    expect_total: u64,
    on_event: &Channel<DownloadProgress>,
) -> Result<(u64, String), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(1800))
        .build()
        .map_err(|e| format!("创建下载客户端失败：{e}"))?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|e| format!("下载构建物失败：{e}"))?;
    if !response.status().is_success() {
        return Err(format!("下载构建物失败：HTTP {}", response.status()));
    }
    let total = response.content_length().unwrap_or(expect_total);
    if total > MAX_ARTIFACT_BYTES {
        return Err(format!("构建物体积超过限制：{total} 字节"));
    }
    let mut file = fs::File::create(target).map_err(|e| format!("创建临时文件失败：{e}"))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut downloaded: u64 = 0;
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("读取下载数据失败：{e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("写入临时文件失败：{e}"))?;
        hasher.update(&buffer[..read]);
        downloaded += read as u64;
        if downloaded > MAX_ARTIFACT_BYTES {
            drop(file);
            let _ = fs::remove_file(target);
            return Err("构建物实际体积超过限制，已中止下载".to_string());
        }
        let _ = on_event.send(DownloadProgress {
            stage: "download".to_string(),
            downloaded,
            total,
        });
    }
    file.flush().map_err(|e| format!("写入临时文件失败：{e}"))?;
    drop(file);
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((downloaded, digest))
}

fn safe_extract(archive: &Path, dest: &Path) -> Result<usize, String> {
    let listing = Command::new("tar")
        .arg("-tzf")
        .arg(archive)
        .output()
        .map_err(|e| format!("调用 tar 失败：{e}"))?;
    if !listing.status.success() {
        return Err("构建物不是有效的 tar.gz 压缩包".to_string());
    }
    let names = String::from_utf8_lossy(&listing.stdout);
    let mut count = 0usize;
    for name in names.lines() {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        count += 1;
        if !is_safe_entry(name) {
            return Err(format!("构建物包含非法路径：{name}"));
        }
    }

    let verbose = Command::new("tar")
        .arg("-tvzf")
        .arg(archive)
        .output()
        .map_err(|e| format!("调用 tar 失败：{e}"))?;
    if !verbose.status.success() {
        return Err("无法检查压缩包内容，请重新选择完整的交付包".to_string());
    }
    let details = String::from_utf8_lossy(&verbose.stdout);
    for line in details.lines() {
        if !line.starts_with('-') && !line.starts_with('d') {
            return Err("交付包包含链接或特殊文件，无法解压，请重新打包后重试".to_string());
        }
    }

    fs::create_dir_all(dest).map_err(|e| format!("创建解包目录失败：{e}"))?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .map_err(|e| format!("调用 tar 失败：{e}"))?;
    if !status.success() {
        return Err("解包构建物失败".to_string());
    }
    Ok(count)
}

fn is_safe_entry(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return false;
    }
    let bytes = name.as_bytes();
    if bytes.len() > 2 && bytes[1] == b':' {
        return false;
    }
    !name
        .split(|ch| ch == '/' || ch == '\\')
        .any(|part| part == "..")
}

fn validate_layout(dir: &Path) -> Result<(String, String), String> {
    if !dir.join("bundle.json").is_file() {
        return Err("构建物缺少 bundle.json，可能不是 deploy-desktop 交付包".to_string());
    }
    // 结构、文件清单与摘要都在描述符里校验，客户端不再假设固定目录结构。
    let bundle = crate::commands::bundle::Bundle::load(dir)?;
    bundle.verify_payload()?;
    let version = validate_artifact_version(bundle.version())?.to_string();
    Ok((version, bundle.metadata().commit_short.clone()))
}

fn install_artifact_archive(
    archive: &Path,
    dest: &Path,
    version: &str,
) -> Result<ArtifactBundle, String> {
    let staged = dest.with_file_name(format!(".unpack-{}-{}", std::process::id(), now_millis()));
    let result = (|| {
        safe_extract(archive, &staged)?;
        let (bundle_version, commit) = validate_layout(&staged)?;
        let files = payload_file_count(&staged);
        if bundle_version != version {
            return Err(format!("交付包版本不匹配：选择的版本为 {version}，包内版本为 {bundle_version}，请重新选择或重新构建"));
        }
        let backup = staged.with_extension("previous");
        let had_previous = dest.exists();
        if had_previous {
            fs::rename(dest, &backup).map_err(|error| format!("保留已有交付包失败：{error}"))?;
        }
        if let Err(error) = fs::rename(&staged, dest) {
            if had_previous {
                fs::rename(&backup, dest).map_err(|restore_error| {
                    format!("保存交付包失败：{error}；恢复原交付包失败：{restore_error}")
                })?;
            }
            return Err(format!("保存交付包失败：{error}"));
        }
        if had_previous {
            let _ = fs::remove_dir_all(backup);
        }
        Ok(ArtifactBundle {
            dir: dest.to_string_lossy().to_string(),
            version: bundle_version,
            commit,
            files,
            md5: String::new(),
            size_bytes: 0,
        })
    })();
    let _ = fs::remove_dir_all(staged);
    result
}

#[tauri::command]
pub async fn download_artifact(
    app: AppHandle,
    id: String,
    release: ArtifactRelease,
    on_event: Channel<DownloadProgress>,
) -> Result<ArtifactBundle, String> {
    crate::commands::blocking::run(move || download_artifact_blocking(app, id, release, on_event))
        .await
}

fn download_artifact_blocking(
    app: AppHandle,
    id: String,
    release: ArtifactRelease,
    on_event: Channel<DownloadProgress>,
) -> Result<ArtifactBundle, String> {
    if release.download_url.trim().is_empty() {
        return Err("下载地址为空".to_string());
    }
    let dir = config_dir(&app)?;
    let mut sources = load_sources(&app)?;
    let index = sources
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| "制品源不存在".to_string())?;

    let source = sources[index].clone();
    let download_url = if source.kind == "url" {
        source.download_url.trim().to_string()
    } else {
        release.download_url.trim().to_string()
    };
    validate_download_url(&download_url, &source)?;

    let version = validate_artifact_version(&release.version)?.to_string();
    if source.kind == "url" && url_release(&source)?.version != version {
        return Err("制品源版本已变更，请刷新列表后重新选择".to_string());
    }
    let safe_version = safe_version_key(&version);

    let dest = dir.join("bundles").join(&id).join(&safe_version);
    log::info!(
        target: "artifact",
        "开始下载交付包：来源={id} 版本={version} 运行=#{} 预计大小={} 字节 地址={}",
        release.run_id,
        release.size_bytes,
        crate::logging::safe_url(&download_url)
    );

    let temp = std::env::temp_dir().join(format!("deploy-bundle-{}-{}.tgz", id, safe_version));
    let _ = on_event.send(DownloadProgress {
        stage: "download".to_string(),
        downloaded: 0,
        total: release.size_bytes,
    });
    let downloaded = download_to_file(&download_url, &temp, release.size_bytes, &on_event);
    let (size, digest) = match downloaded {
        Ok(result) => result,
        Err(error) => {
            let _ = fs::remove_file(&temp);
            log::error!(target: "artifact", "下载交付包失败：来源={id} 版本={version}：{error}");
            return Err(error);
        }
    };
    log::info!(
        target: "artifact",
        "下载完成：来源={id} 版本={version} 实际大小={size} 字节 MD5={digest}"
    );

    if !release.md5.trim().is_empty() && !digest.eq_ignore_ascii_case(release.md5.trim()) {
        let _ = fs::remove_file(&temp);
        log::error!(
            target: "artifact",
            "构建物 MD5 不一致：来源={id} 版本={version} 期望={} 实际={digest}",
            release.md5
        );
        return Err(format!(
            "构建物校验失败：期望 MD5 {}，实际 {}",
            release.md5, digest
        ));
    }

    let _ = on_event.send(DownloadProgress {
        stage: "extract".to_string(),
        downloaded: size,
        total: size,
    });
    log::info!(target: "artifact", "开始校验并解包交付包：来源={id} 版本={version}");
    let installed = install_artifact_archive(&temp, &dest, &version);
    let _ = fs::remove_file(&temp);
    let mut bundle = match installed {
        Ok(bundle) => bundle,
        Err(error) => {
            log::error!(target: "artifact", "交付包校验或解包失败：来源={id} 版本={version}：{error}");
            return Err(error);
        }
    };
    log::info!(
        target: "artifact",
        "交付包已就绪：来源={id} 版本={} 目录={} 文件数={}",
        bundle.version,
        bundle.dir,
        bundle.files
    );
    sources[index].last_version = bundle.version.clone();
    sources[index].last_bundle_dir = dest.to_string_lossy().to_string();
    store_sources(&app, &sources)?;

    let _ = on_event.send(DownloadProgress {
        stage: "done".to_string(),
        downloaded: size,
        total: size,
    });

    if bundle.commit.is_empty() {
        bundle.commit = release.commit_short;
    }
    bundle.md5 = digest;
    bundle.size_bytes = size;
    Ok(bundle)
}

fn count_files(dir: &Path) -> usize {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                total += count_files(&path);
            } else if path.is_file() {
                total += 1;
            }
        }
    }
    total
}

/// 部署内容文件数：交给界面的数字与部署计划保持一致，不含描述文件本身。
fn payload_file_count(dir: &Path) -> usize {
    match crate::commands::bundle::Bundle::load(dir) {
        Ok(bundle) => bundle.payload_file_count(),
        Err(_) => count_files(dir),
    }
}

// 本地目录与压缩包共用同一结构检查；不跟随符号链接，避免部署时访问包外文件。
fn inspect_local_tree(dir: &Path, depth: usize) -> Result<(usize, u64), String> {
    if depth > 128 {
        return Err("交付包目录层级过深，请检查后重试".to_string());
    }
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in fs::read_dir(dir).map_err(|e| format!("读取交付包目录失败：{e}"))? {
        let entry = entry.map_err(|e| format!("读取交付包文件失败：{e}"))?;
        let meta = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("检查交付包文件失败：{e}"))?;
        if meta.is_dir() {
            let (nested_count, nested_bytes) = inspect_local_tree(&entry.path(), depth + 1)?;
            count += nested_count;
            bytes = bytes.saturating_add(nested_bytes);
        } else if meta.is_file() {
            count += 1;
            bytes = bytes.saturating_add(meta.len());
        } else {
            return Err("交付包包含链接或特殊文件，请选择仅含普通文件的交付包".to_string());
        }
        if count > 200_000 || bytes > MAX_ARTIFACT_BYTES * 4 {
            return Err("交付包解压后体积或文件数量超过限制，请检查后重试".to_string());
        }
    }
    Ok((count, bytes))
}

fn prepare_local_bundle(path: &Path, cache_root: &Path) -> Result<ArtifactBundle, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("读取所选制品失败：{e}"))?;
    if meta.file_type().is_symlink() {
        return Err("请选择交付包原文件或目录，不能使用符号链接".to_string());
    }
    let source = fs::canonicalize(path).map_err(|e| format!("读取所选制品失败：{e}"))?;
    let extracted = if meta.is_dir() {
        None
    } else if meta.is_file() {
        let name = source.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if !name.ends_with(".tgz") && !name.ends_with(".tar.gz") {
            return Err("请选择 .tgz、.tar.gz 交付包，或已解压的交付包目录".to_string());
        }
        if meta.len() > MAX_ARTIFACT_BYTES {
            return Err("交付包超过 5 GiB，请检查所选文件".to_string());
        }
        fs::create_dir_all(cache_root).map_err(|e| format!("创建交付包目录失败：{e}"))?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "无法生成交付包目录，请检查系统时间".to_string())?.as_nanos();
        let dest = cache_root.join(format!("local-{}-{stamp}", std::process::id()));
        fs::create_dir(&dest).map_err(|e| format!("创建解压目录失败：{e}"))?;
        if let Err(error) = safe_extract(&source, &dest) {
            let _ = fs::remove_dir_all(&dest);
            return Err(error);
        }
        Some(dest)
    } else {
        return Err("所选制品不是普通文件或目录，请重新选择".to_string());
    };
    let dir = extracted.as_deref().unwrap_or(&source);
    let result = (|| {
        let (_, bytes) = inspect_local_tree(dir, 0)?;
        let (version, commit) = validate_layout(dir)?;
        let files = payload_file_count(dir);
        Ok(ArtifactBundle {
            dir: dir.to_string_lossy().to_string(), version, commit,
            md5: String::new(), size_bytes: bytes, files,
        })
    })();
    if result.is_err() {
        if let Some(dest) = extracted { let _ = fs::remove_dir_all(dest); }
    }
    result
}

#[tauri::command]
pub async fn prepare_local_artifact_bundle(app: AppHandle, path: String) -> Result<ArtifactBundle, String> {
    if path.trim().is_empty() {
        return Err("请先选择交付包文件或目录".to_string());
    }
    let cache_root = config_dir(&app)?.join("bundles/local");
    tauri::async_runtime::spawn_blocking(move || prepare_local_bundle(Path::new(path.trim()), &cache_root))
        .await.map_err(|e| format!("制品准备失败，请重试：{e}"))?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBundleRemoval {
    pub dir: String,
    pub freed_bytes: u64,
}

fn dir_size_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.metadata() {
            Ok(meta) if meta.is_dir() => total += dir_size_bytes(&path),
            Ok(meta) => total += meta.len(),
            Err(_) => {}
        }
    }
    total
}

/// 删除某个制品源在本机缓存的全部交付包。
///
/// 只允许删除应用配置目录下 bundles/<id> 的内容；删除后清空索引里的
/// lastVersion / lastBundleDir，界面上该行不再显示"使用已下载"。
#[tauri::command]
pub async fn delete_local_bundle(
    app: AppHandle,
    id: String,
) -> Result<LocalBundleRemoval, String> {
    crate::commands::blocking::run(move || delete_local_bundle_blocking(app, id)).await
}

fn delete_local_bundle_blocking(app: AppHandle, id: String) -> Result<LocalBundleRemoval, String> {
    let id = id.trim().to_string();
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("非法的制品源标识".to_string());
    }

    let mut sources = load_sources(&app)?;
    let index = sources
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| "制品源不存在".to_string())?;

    let config_root = config_dir(&app)?;
    let removal = remove_bundle_tree(&config_root, &id)?;

    sources[index].last_version.clear();
    sources[index].last_bundle_dir.clear();
    store_sources(&app, &sources)?;
    super::projects::clear_bundle_dir(&config_root.join("bundles").join(&id));

    Ok(removal)
}

fn remove_bundle_tree(config_root: &Path, id: &str) -> Result<LocalBundleRemoval, String> {
    let root = config_root.join("bundles").join(id);
    if !root.is_dir() {
        return Err(format!("本地缓存目录不存在：{}", root.display()));
    }

    let freed_bytes = dir_size_bytes(&root);
    fs::remove_dir_all(&root).map_err(|e| format!("删除本地交付包失败：{e}"))?;

    Ok(LocalBundleRemoval {
        dir: root.to_string_lossy().to_string(),
        freed_bytes,
    })
}

#[tauri::command]
pub fn open_artifact_bundle(app: AppHandle, id: String) -> Result<ArtifactBundle, String> {
    let sources = load_sources(&app)?;
    let source = sources
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "制品源不存在".to_string())?;
    if source.last_bundle_dir.trim().is_empty() {
        return Err("该制品源还没有下载过交付包".to_string());
    }
    let dir = PathBuf::from(source.last_bundle_dir.trim());
    if !dir.is_dir() {
        return Err(format!("本地交付包目录不存在：{}", dir.display()));
    }

    let (version, commit) = validate_layout(&dir)?;

    Ok(ArtifactBundle {
        dir: dir.to_string_lossy().to_string(),
        version,
        commit,
        md5: String::new(),
        size_bytes: 0,
        files: payload_file_count(&dir),
    })
}

#[tauri::command]
pub fn open_artifact_bundle_version(
    app: AppHandle,
    id: String,
    version: String,
) -> Result<ArtifactBundle, String> {
    let id = id.trim().to_string();
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("非法的制品源标识".to_string());
    }

    let config_root = config_dir(&app)?;
    let mut sources = load_sources(&app)?;
    let index = sources
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| "制品源不存在".to_string())?;
    let source = sources[index].clone();

    let version = validate_artifact_version(&version)?;
    let mut dir = config_root
        .join("bundles")
        .join(&id)
        .join(safe_version_key(version));
    if !dir.is_dir()
        && !source.last_bundle_dir.trim().is_empty()
        && source.last_version.trim() == version
    {
        dir = PathBuf::from(source.last_bundle_dir.trim());
    }
    if !dir.is_dir() {
        return Err(format!("版本 {version} 尚未下载到本机"));
    }

    let (bundle_version, commit) = validate_layout(&dir)?;
    if bundle_version != version {
        log::error!(
            target: "artifact",
            "本地交付包版本不一致：目录={} 期望={version} 实际={bundle_version}",
            dir.display()
        );
        return Err("本地交付包版本与所选版本不一致，请重新下载该版本".to_string());
    }
    sources[index].last_version = bundle_version.clone();
    sources[index].last_bundle_dir = dir.to_string_lossy().to_string();
    store_sources(&app, &sources)?;

    Ok(ArtifactBundle {
        dir: dir.to_string_lossy().to_string(),
        version: bundle_version,
        commit,
        md5: String::new(),
        size_bytes: 0,
        files: payload_file_count(&dir),
    })
}

#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "链接格式无效".to_string())?;
    if parsed.scheme() != "https" {
        return Err("仅允许打开 HTTPS 链接".to_string());
    }
    let host = parsed.host_str().unwrap_or_default();
    let allowed = host == "flow.aliyun.com"
        || host.ends_with(".aliyun.com")
        || host.ends_with(".aliyuncs.com");
    if !allowed {
        return Err("链接域名不在允许范围内".to_string());
    }

    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(parsed.as_str()).status();
    #[cfg(target_os = "windows")]
    let status = Command::new("cmd")
        .args(["/C", "start", "", parsed.as_str()])
        .status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = Command::new("xdg-open").arg(parsed.as_str()).status();

    status.map_err(|e| format!("打开浏览器失败：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 按目录实际内容生成 v2 bundle.json，避免测试手写文件摘要。
    fn write_v2_bundle(dir: &Path, version: &str) {
        let mut files: Vec<serde_json::Value> = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            let Ok(entries) = fs::read_dir(&current) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = path.strip_prefix(dir).unwrap().to_string_lossy().to_string();
                if relative == "bundle.json" {
                    continue;
                }
                let bytes = fs::read(&path).unwrap();
                files.push(json!({
                    "path": relative,
                    "size": bytes.len(),
                    "sha256": crate::commands::bundle::sha256_hex(&bytes),
                }));
            }
        }
        files.sort_by(|left, right| {
            left["path"].as_str().unwrap_or_default().cmp(right["path"].as_str().unwrap_or_default())
        });
        let manifest = json!({
            "apiVersion": "deploy.desktop/bundle/v2",
            "kind": "DeploymentBundle",
            "metadata": { "product": "demo", "version": version, "commitShort": "test",
                "commit": "", "branch": "main", "builtAt": "" },
            "runtime": { "type": "compose", "compose": "deploy/docker-compose.yml",
                "containerPrefix": "demo", "services": [] },
            "components": [{ "key": "app", "label": "应用", "artifacts": ["jar", "dist"], "services": [] }],
            "artifacts": [
                { "key": "jar", "source": "app/server.jar", "root": "install",
                  "path": "server/server.jar", "type": "file" },
                { "key": "dist", "source": "app/frontend-dist", "root": "deploy",
                  "path": "frontend_dist", "type": "dir" },
                { "key": "compose", "source": "deploy/docker-compose.yml", "root": "deploy",
                  "path": "docker-compose.yml", "type": "file" }
            ],
            "images": [],
            "plan": { "steps": [] },
            "files": files,
        });
        fs::write(dir.join("bundle.json"), serde_json::to_string(&manifest).unwrap()).unwrap();
    }

    #[test]
    fn local_artifact_archive_is_prepared_without_overwriting_source() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("local-artifact-{}-{stamp}", std::process::id()));
        let source = root.join("source");
        fs::create_dir_all(source.join("app/frontend-dist")).unwrap();
        fs::create_dir_all(source.join("deploy")).unwrap();
        fs::write(source.join("app/server.jar"), "artifact").unwrap();
        fs::write(source.join("app/frontend-dist/index.html"), "frontend").unwrap();
        fs::write(source.join("deploy/docker-compose.yml"), "services: {}\n").unwrap();
        write_v2_bundle(&source, "2");
        let archive = root.join("bundle.tgz");
        assert!(Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&source).arg(".").status().unwrap().success());
        let prepared = prepare_local_bundle(&archive, &root.join("cache")).unwrap();
        assert_eq!(prepared.version, "2");
        assert_eq!(prepared.files, 3);
        assert!(Path::new(&prepared.dir).join("deploy/docker-compose.yml").is_file());
        assert_eq!(fs::read_to_string(source.join("app/server.jar")).unwrap(), "artifact");
        let cache_count = fs::read_dir(root.join("cache")).unwrap().count();
        fs::remove_file(source.join("bundle.json")).unwrap();
        let invalid = root.join("invalid.tgz");
        assert!(Command::new("tar").arg("-czf").arg(&invalid).arg("-C").arg(&source).arg(".").status().unwrap().success());
        assert!(prepare_local_bundle(&invalid, &root.join("cache")).is_err());
        assert_eq!(fs::read_dir(root.join("cache")).unwrap().count(), cache_count);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_bundle_is_rejected_with_regeneration_hint() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("legacy-artifact-{}-{stamp}", std::process::id()));
        fs::create_dir_all(root.join("deploy")).unwrap();
        fs::write(root.join("deploy/docker-compose.yml"), "services: {}\n").unwrap();
        fs::write(root.join("bundle.json"), r#"{"version":"7","mode":"file","components":["backend"]}"#).unwrap();
        assert!(prepare_local_bundle(&root, &root.join("cache")).is_err());
        assert!(!root.join("cache").exists() || fs::read_dir(root.join("cache")).unwrap().next().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_artifact_rejects_links_before_reading_package_files() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("local-artifact-link-{stamp}"));
        fs::create_dir_all(root.join("source")).unwrap();
        fs::write(root.join("outside.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(root.join("outside.txt"), root.join("source/link")).unwrap();
        assert!(prepare_local_bundle(&root.join("source"), &root.join("cache")).is_err());
        let archive = root.join("bundle.tgz");
        assert!(Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(root.join("source")).arg(".").status().unwrap().success());
        assert!(prepare_local_bundle(&archive, &root.join("cache")).is_err());
        assert_eq!(fs::read_to_string(root.join("outside.txt")).unwrap(), "outside");
        assert!(fs::read_dir(root.join("cache")).unwrap().next().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    fn source(kind: &str, url: &str) -> ArtifactSource {
        ArtifactSource {
            kind: kind.to_string(),
            download_url: url.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn rejects_plain_http() {
        let item = source("yunxiaoFlow", "");
        assert!(validate_download_url("http://example.com/a.tgz", &item).is_err());
    }

    #[test]
    fn allows_aliyun_hosts_for_flow_source() {
        let item = source("yunxiaoFlow", "");
        assert!(validate_download_url("https://flow.aliyuncs.com/a.tgz", &item).is_ok());
        assert!(validate_download_url("https://oss-cn-hangzhou.aliyuncs.com/a.tgz", &item).is_ok());
    }

    #[test]
    fn rejects_foreign_host_for_flow_source() {
        let item = source("yunxiaoFlow", "");
        assert!(validate_download_url("https://example.com/a.tgz", &item).is_err());
    }

    #[test]
    fn url_source_must_match_configured_address() {
        let item = source("url", "https://files.example.com/bundle.tgz");
        assert!(validate_download_url("https://files.example.com/bundle.tgz", &item).is_ok());
        assert!(validate_download_url("https://files.example.com/other.tgz", &item).is_err());
    }

    #[test]
    fn md5_matches_known_vector() {
        use md5::{Digest, Md5};
        let digest: String = Md5::digest(b"abc")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(digest, "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn extracts_artifact_from_nested_job_result() {
        let inner = serde_json::json!({
            "artifact": "deploy-bundle.tgz",
            "md5": "9de586e2c53cf541d46f118f31007f6c",
            "version": "5",
            "repoId": "flow_generic_repo",
            "downloadUrl": "https://packages.aliyun.com/api/protocol/org/generic/repo/files/deploy-bundle",
            "type": "packages"
        });
        let outer = serde_json::json!({
            "data": {
                "build_stage-build_job-upload_step.dynamic_output.deploy-bundle": inner.to_string()
            }
        });
        let detail = serde_json::json!({
            "status": "SUCCESS",
            "stages": [{
                "stageInfo": {
                    "startTime": 1792405759000i64,
                    "jobs": [{ "result": outer.to_string() }]
                }
            }],
            "outputsToEnv.CI_COMMIT_ID": "952e9dc"
        });

        let artifact = pick_artifact(&detail, "deploy-bundle").expect("应能取到制品");
        assert_eq!(json_str(&artifact, "version"), "5");
        assert_eq!(json_str(&artifact, "md5"), "9de586e2c53cf541d46f118f31007f6c");
        assert_eq!(find_key_i64(&detail, "startTime"), 1792405759000);
        assert_eq!(
            find_key_string(&detail, "CI_COMMIT_ID").as_deref(),
            Some("952e9dc")
        );
        assert!(validate_download_url(json_str(&artifact, "downloadUrl"), &source("yunxiaoFlow", "")).is_ok());
    }

    #[test]
    fn flow_release_requires_matching_name_and_version() {
        let mut item = source("yunxiaoFlow", "");
        item.artifact_name = "deploy-bundle".to_string();
        let mut detail = serde_json::json!({
            "status": "SUCCESS",
            "artifacts": [{
                "artifact": "deploy-bundle-other.tgz",
                "version": "8-a1b2c3d",
                "downloadUrl": "https://packages.aliyun.com/bundle.tgz",
                "md5": "abc",
                "type": "packages"
            }]
        });
        assert!(release_from_flow_detail(&item, 8, &detail).unwrap().is_none());
        detail["artifacts"][0]["artifact"] = "deploy-bundle.tgz".into();
        let release = release_from_flow_detail(&item, 8, &detail).unwrap().unwrap();
        assert_eq!(release.version, "8-a1b2c3d");
        detail["artifacts"][0]["version"] = 7.into();
        assert_eq!(release_from_flow_detail(&item, 7, &detail).unwrap().unwrap().version, "7");
        detail["artifacts"][0]["version"] = Value::Null;
        assert!(release_from_flow_detail(&item, 8, &detail).unwrap_err().contains("缺少版本号"));
    }

    #[test]
    fn url_release_uses_configured_version_instead_of_cached_version() {
        let mut item = source("url", "https://files.example.com/bundle.tgz");
        assert!(url_release(&item).is_err());
        item.last_version = "7".to_string();
        assert_eq!(url_release(&item).unwrap().version, "7");
        item.version = "8-a1b2c3d".to_string();
        assert_eq!(url_release(&item).unwrap().version, "8-a1b2c3d");
        for invalid in ["", "manual", "..", "../8", "/8", "8/other", "8\\other"] {
            assert!(validate_artifact_version(invalid).is_err());
        }
    }

    #[test]
    fn mismatched_archive_keeps_previous_cache_and_is_not_marked_downloaded() {
        let root = std::env::temp_dir().join(format!("deploy-version-check-{}", std::process::id()));
        let fixture = root.join("fixture");
        let cached = root.join("bundles/src-test/8-a1b2c3d");
        fs::create_dir_all(fixture.join("app/frontend-dist")).unwrap();
        fs::create_dir_all(fixture.join("deploy")).unwrap();
        fs::create_dir_all(&cached).unwrap();
        fs::write(cached.join("previous.txt"), "preserve").unwrap();
        fs::write(fixture.join("app/server.jar"), "jar").unwrap();
        fs::write(fixture.join("app/frontend-dist/index.html"), "html").unwrap();
        fs::write(fixture.join("deploy/docker-compose.yml"), "services: {}").unwrap();
        write_v2_bundle(&fixture, "7");
        let archive = root.join("bundle.tgz");
        let status = Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&fixture)
            .args(["app", "deploy", "bundle.json"]).status().unwrap();
        assert!(status.success());
        let error = install_artifact_archive(&archive, &cached, "8-a1b2c3d").unwrap_err();
        assert!(error.contains("版本不匹配"));
        assert_eq!(fs::read_to_string(cached.join("previous.txt")).unwrap(), "preserve");

        let mut item = source("yunxiaoFlow", "");
        item.id = "src-test".to_string();
        let release = ArtifactRelease { version: "8-a1b2c3d".to_string(), ..Default::default() };
        assert!(cached_bundle_dir(&root, &item, &release).is_none());
        write_v2_bundle(&fixture, "8-a1b2c3d");
        assert!(Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&fixture)
            .args(["app", "deploy", "bundle.json"]).status().unwrap().success());
        let installed = install_artifact_archive(&archive, &cached, "8-a1b2c3d").unwrap();
        assert_eq!(installed.version, "8-a1b2c3d");
        assert!(!cached.join("previous.txt").exists());
        assert_eq!(cached_bundle_dir(&root, &item, &release), Some(cached));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unsafe_archive_entries() {
        assert!(is_safe_entry("app/server.jar"));
        assert!(is_safe_entry("bundle.json"));
        assert!(!is_safe_entry("/etc/passwd"));
        assert!(!is_safe_entry("../../etc/passwd"));
        assert!(!is_safe_entry("app/../../x"));
        assert!(!is_safe_entry("C:/Windows/system32"));
        assert!(!is_safe_entry("..\\x"));
        assert!(!is_safe_entry(""));
    }

    #[test]
    fn removes_only_the_requested_bundle_tree() {
        let root = std::env::temp_dir().join(format!(
            "deploy-desktop-bundles-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let target = root.join("bundles/src-a/5");
        let sibling = root.join("bundles/src-b/9");
        fs::create_dir_all(target.join("deploy")).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        fs::write(target.join("bundle.json"), vec![0u8; 2048]).unwrap();
        fs::write(target.join("deploy/docker-compose.yml"), b"name: x").unwrap();

        let removal = remove_bundle_tree(&root, "src-a").unwrap();
        assert_eq!(removal.freed_bytes, 2048 + 7);
        assert!(!root.join("bundles/src-a").exists());
        assert!(sibling.is_dir());
        assert!(remove_bundle_tree(&root, "src-a").is_err());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tokens_persist_in_local_file() {
        let dir = std::env::temp_dir().join(format!(
            "deploy-desktop-tokens-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let id = format!("src-test-{}", std::process::id());

        secret_set(&dir, &id, "token-1").unwrap();
        assert_eq!(secret_get(&dir, &id).unwrap().as_deref(), Some("token-1"));
        assert!(secrets_file(&dir).is_file());

        secret_delete(&dir, &id).unwrap();
        let mut secrets = read_secrets(&dir);
        secrets.insert(keychain_marker(&id), "1".to_string());
        write_secrets(&dir, &secrets).unwrap();
        assert_eq!(secret_get(&dir, &id).unwrap(), None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn token_library_round_trips_and_resolves_sources() {
        let root = cache_root("token-library");
        let token = ArtifactToken {
            id: "tok-1".to_string(),
            name: "云效个人令牌".to_string(),
            created_at: 1,
            updated_at: 2,
        };
        secret_set(&root, &token_secret_key(&token.id), "secret-1").unwrap();
        write_token_library(&root, &[token.clone()]).unwrap();

        let loaded = read_token_library(&root).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "云效个人令牌");
        assert_eq!(loaded[0].updated_at, 2);

        let mut source = cache_source();
        assert_eq!(source_token(&root, &source).unwrap(), None);
        source.token_id = "tok-1".to_string();
        assert_eq!(
            source_token(&root, &source).unwrap().as_deref(),
            Some("secret-1")
        );

        secret_delete(&root, &token_secret_key("tok-1")).unwrap();
        assert_eq!(source_token(&root, &source).unwrap(), None);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_token_names_get_suffix() {
        let tokens = vec![ArtifactToken {
            id: "tok-1".to_string(),
            name: "云效".to_string(),
            ..Default::default()
        }];
        assert_eq!(unique_token_name(&tokens, "生产"), "生产");
        assert_eq!(unique_token_name(&tokens, "云效"), "云效 2");
    }

    #[cfg(unix)]
    #[test]
    fn token_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "deploy-desktop-token-perm-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        secret_set(&dir, "src-perm", "token").unwrap();
        let mode = fs::metadata(secrets_file(&dir)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(&dir);
    }

    fn cache_source() -> ArtifactSource {
        ArtifactSource {
            id: "src-cache".to_string(),
            name: "缓存测试".to_string(),
            kind: "yunxiaoFlow".to_string(),
            org_id: "org".to_string(),
            pipeline_id: "pipeline".to_string(),
            artifact_name: "deploy-bundle".to_string(),
            ..Default::default()
        }
    }

    fn cache_root(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "deploy-desktop-{label}-{}-{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn cache_page(items: Vec<ArtifactRelease>, page: u32, has_more: bool) -> ArtifactReleasePage {
        ArtifactReleasePage {
            items,
            page,
            per_page: FLOW_RUN_PAGE_SIZE,
            has_more,
        }
    }

    #[test]
    fn cached_release_page_shows_stored_and_local_versions() {
        let root = cache_root("release-cache");
        let source = cache_source();
        store_release_cache(
            &root,
            &source,
            &cache_page(
                vec![ArtifactRelease {
                    run_id: 7,
                    version: "7".to_string(),
                    artifact_name: "deploy-bundle".to_string(),
                    built_at: 1000,
                    download_url: "https://example.aliyuncs.com/bundle?signature=secret".to_string(),
                    ..Default::default()
                }],
                1,
                true,
            ),
        );
        let local = root
            .join("bundles")
            .join(&source.id)
            .join("8");
        fs::create_dir_all(&local).unwrap();
        fs::write(
            local.join("bundle.json"),
            r#"{"metadata":{"version":"8","commit":"abcdef0123","commitShort":"abcdef0","branch":"master","builtAt":"2026-10-19T00:00:00Z"},"files":[{"path":"a","size":2048}]}"#,
        )
        .unwrap();
        let legacy = root.join("bundles").join(&source.id).join("6");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("bundle.json"),
            r#"{"schemaVersion":1,"version":"6","builtAt":"2026-10-18T00:00:00Z"}"#,
        )
        .unwrap();

        let cached = cached_release_page(&root, &source);
        assert_eq!(cached.items.len(), 2);
        assert_eq!(cached.items[0].version, "8");
        assert_eq!(cached.items[0].local_dir, local.to_string_lossy());
        assert_eq!(cached.items[0].commit_short, "abcdef0");
        assert_eq!(cached.items[0].size_bytes, 2048);
        assert_eq!(cached.items[0].built_at, 1_792_368_000_000);
        assert!(cached.items.iter().all(|item| item.version != "6"));
        assert_eq!(cached.items[1].version, "7");
        assert_eq!(cached.items[1].download_url, "");
        assert!(cached.has_more);

        let text = fs::read_to_string(release_cache_path(&root)).unwrap();
        assert!(!text.contains("signature=secret"));

        let mut changed = source.clone();
        changed.pipeline_id = "other-pipeline".to_string();
        assert!(cached_release_page(&root, &changed)
            .items
            .iter()
            .all(|item| item.version != "7"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cached_release_history_keeps_older_loaded_pages() {
        let root = cache_root("release-pages");
        let source = cache_source();
        store_release_cache(
            &root,
            &source,
            &cache_page(
                vec![ArtifactRelease {
                    run_id: 8,
                    version: "8".to_string(),
                    built_at: 2000,
                    ..Default::default()
                }],
                1,
                true,
            ),
        );
        store_release_cache(
            &root,
            &source,
            &cache_page(
                vec![ArtifactRelease {
                    run_id: 7,
                    version: "7".to_string(),
                    built_at: 1000,
                    ..Default::default()
                }],
                2,
                false,
            ),
        );

        let cached = cached_release_page(&root, &source);
        assert_eq!(
            cached.items.iter().map(|item| item.version.as_str()).collect::<Vec<_>>(),
            vec!["8", "7"]
        );
        assert!(!cached.has_more);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_bundle_build_times_with_timezone_offsets() {
        assert_eq!(parse_rfc3339_millis("2026-10-19T00:00:00Z"), Some(1_792_368_000_000));
        assert_eq!(parse_rfc3339_millis("2026-10-19T08:00:00+08:00"), Some(1_792_368_000_000));
        assert_eq!(parse_rfc3339_millis("not-a-time"), None);
    }
}
