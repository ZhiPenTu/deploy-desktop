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
    pub download_url: String,
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
    pub download_url: String,
    pub token: Option<String>,
    pub clear_token: bool,
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

#[tauri::command]
pub fn list_artifact_sources(app: AppHandle) -> Result<Vec<ArtifactSource>, String> {
    load_sources(&app)
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

    if source.clear_token {
        secret_delete(&dir, &id)?;
    } else if let Some(token) = source.token.as_ref().map(|v| v.trim()).filter(|v| !v.is_empty()) {
        secret_set(&dir, &id, token)?;
    }

    let has_token = secret_get(&dir, &id)?
        .map(|value| !value.is_empty())
        .unwrap_or(false);

    let record = ArtifactSource {
        id: id.clone(),
        name,
        kind,
        org_id: source.org_id.trim().to_string(),
        pipeline_id: source.pipeline_id.trim().to_string(),
        artifact_name: if source.artifact_name.trim().is_empty() {
            "deploy-bundle".to_string()
        } else {
            source.artifact_name.trim().to_string()
        },
        download_url: source.download_url.trim().to_string(),
        has_token,
        last_version,
        last_bundle_dir,
    };

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
        .map_err(|e| format!("请求云效接口失败：{e}"))?;
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
                        || name.starts_with(artifact_name)
                })
                .unwrap_or(false)
        })
        .or_else(|| {
            candidates.iter().find(|item| {
                item.get("type").and_then(Value::as_str) == Some("packages")
            })
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

fn url_release(source: &ArtifactSource) -> ArtifactRelease {
    ArtifactRelease {
        run_id: 0,
        run_url: String::new(),
        run_status: "manual".to_string(),
        version: if source.last_version.trim().is_empty() {
            "manual".to_string()
        } else {
            source.last_version.clone()
        },
        artifact_name: source.artifact_name.clone(),
        download_url: source.download_url.clone(),
        ..Default::default()
    }
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

    Ok(Some(ArtifactRelease {
        run_id,
        run_url: format!("{FLOW_WEB}/{}/runs/{}", source.pipeline_id, run_id),
        run_status: json_str(detail, "status").to_string(),
        version: json_str(&artifact, "version").to_string(),
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
    let version = if source.kind == "url" {
        "manual".to_string()
    } else if release.version.trim().is_empty() {
        "manual".to_string()
    } else {
        safe_version_key(&release.version)
    };
    let expected = config_root.join("bundles").join(&source.id).join(version);
    if expected.is_dir() {
        return Some(expected);
    }

    let last_dir = source.last_bundle_dir.trim();
    if last_dir.is_empty()
        || (source.kind != "url" && source.last_version.trim() != release.version.trim())
    {
        return None;
    }
    let last_dir = PathBuf::from(last_dir);
    last_dir.is_dir().then_some(last_dir)
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

    Ok(ArtifactReleasePage {
        items: releases,
        page: page.max(1),
        per_page: FLOW_RUN_PAGE_SIZE,
        has_more: runs.len() >= FLOW_RUN_PAGE_SIZE as usize,
    })
}

fn load_artifact_release_page(
    config_root: &Path,
    source: &ArtifactSource,
    page: u32,
) -> Result<ArtifactReleasePage, String> {
    if source.kind == "url" {
        let mut release = url_release(source);
        attach_local_bundle(config_root, source, &mut release);
        return Ok(ArtifactReleasePage {
            items: vec![release],
            page: 1,
            per_page: 1,
            has_more: false,
        });
    }

    let token = secret_get(config_root, &source.id)?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "尚未配置云效个人访问令牌，请编辑制品源填写令牌".to_string())?;
    load_flow_release_page(config_root, source, &token, page.max(1))
}

#[tauri::command]
pub fn list_artifact_releases(
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
pub fn fetch_artifact_release(
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
        let mut release = url_release(source);
        attach_local_bundle(&config_root, source, &mut release);
        return Ok(release);
    }

    let token = secret_get(&config_root, &source.id)?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "尚未配置云效个人访问令牌，请编辑制品源填写令牌".to_string())?;

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

fn md5_hex(data: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("读取下载数据失败：{e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("写入临时文件失败：{e}"))?;
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
    let mut content = fs::read(target).map_err(|e| format!("读取临时文件失败：{e}"))?;
    let digest = md5_hex(&content);
    content.clear();
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
    let details = String::from_utf8_lossy(&verbose.stdout);
    for line in details.lines() {
        if line.starts_with('l') || line.starts_with('h') {
            return Err("构建物包含符号链接，已拒绝解包".to_string());
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

fn has_image_archive(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                let name = entry
                    .file_name()
                    .to_string_lossy()
                    .to_lowercase();
                entry.path().is_file()
                    && (name.ends_with(".tar")
                        || name.ends_with(".tgz")
                        || name.ends_with(".tar.gz"))
            })
        })
        .unwrap_or(false)
}

fn validate_layout(dir: &Path, backend_artifact: &str) -> Result<(String, String), String> {
    let bundle_json = dir.join("bundle.json");
    if !bundle_json.is_file() {
        return Err("构建物缺少 bundle.json，可能不是 deploy-desktop 交付包".to_string());
    }
    let text = fs::read_to_string(&bundle_json).map_err(|e| format!("读取 bundle.json 失败：{e}"))?;
    let meta: Value = serde_json::from_str(&text).map_err(|e| format!("bundle.json 格式无效：{e}"))?;

    let mut missing = Vec::new();
    // 两种交付形态都接受：
    //   - 文件模式：app/ + deploy/（逐文件替换 jar、classes、lib、dist）
    //   - 镜像模式：images/ + deploy/（docker load 镜像后由 compose 直接使用）
    let image_mode = dir.join("images").is_dir() && !dir.join("app").is_dir();
    if !image_mode {
        if !dir.join("app").join(backend_artifact).is_file() {
            missing.push(format!("app/{backend_artifact}"));
        }
        if !dir.join("app/frontend-dist/index.html").is_file() {
            missing.push("app/frontend-dist/index.html".to_string());
        }
    } else if !has_image_archive(&dir.join("images")) {
        missing.push("images/*.tar.gz".to_string());
    }
    if !dir.join("deploy/docker-compose.yml").is_file() {
        missing.push("deploy/docker-compose.yml".to_string());
    }
    if !missing.is_empty() {
        return Err(format!("交付包缺少必要文件：{}", missing.join("、")));
    }

    Ok((
        json_str(&meta, "version").to_string(),
        json_str(&meta, "commitShort").to_string(),
    ))
}

#[tauri::command]
pub fn download_artifact(
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

    let profile = crate::commands::profile::deployment_profile();
    let version = if source.kind == "url" || release.version.trim().is_empty() {
        "manual".to_string()
    } else {
        release.version.trim().to_string()
    };
    let safe_version = safe_version_key(&version);

    let dest = dir.join("bundles").join(&id).join(&safe_version);
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧解包目录失败：{e}"))?;
    }

    let temp = std::env::temp_dir().join(format!("deploy-bundle-{}-{}.tgz", id, safe_version));
    let _ = on_event.send(DownloadProgress {
        stage: "download".to_string(),
        downloaded: 0,
        total: release.size_bytes,
    });
    let (size, digest) = download_to_file(&download_url, &temp, release.size_bytes, &on_event)?;

    if !release.md5.trim().is_empty() && !digest.eq_ignore_ascii_case(release.md5.trim()) {
        let _ = fs::remove_file(&temp);
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
    let count = safe_extract(&temp, &dest)?;
    let _ = fs::remove_file(&temp);

    let (bundle_version, bundle_commit) = validate_layout(&dest, &profile.backend_artifact)?;

    sources[index].last_version = if bundle_version.is_empty() {
        version
    } else {
        bundle_version.clone()
    };
    sources[index].last_bundle_dir = dest.to_string_lossy().to_string();
    store_sources(&app, &sources)?;

    let _ = on_event.send(DownloadProgress {
        stage: "done".to_string(),
        downloaded: size,
        total: size,
    });

    Ok(ArtifactBundle {
        dir: dest.to_string_lossy().to_string(),
        version: bundle_version,
        commit: if bundle_commit.is_empty() { release.commit_short } else { bundle_commit },
        md5: digest,
        size_bytes: size,
        files: count,
    })
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
pub fn delete_local_bundle(app: AppHandle, id: String) -> Result<LocalBundleRemoval, String> {
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

    let profile = crate::commands::profile::deployment_profile();
    let (version, commit) = validate_layout(&dir, &profile.backend_artifact)?;

    Ok(ArtifactBundle {
        dir: dir.to_string_lossy().to_string(),
        version,
        commit,
        md5: String::new(),
        size_bytes: 0,
        files: count_files(&dir),
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

    let version = version.trim();
    if version.is_empty() {
        return Err("缺少制品版本".to_string());
    }
    let mut dir = config_root
        .join("bundles")
        .join(&id)
        .join(safe_version_key(version));
    if !dir.is_dir() && source.kind == "url" {
        dir = config_root.join("bundles").join(&id).join("manual");
    }
    if !dir.is_dir()
        && !source.last_bundle_dir.trim().is_empty()
        && (source.kind == "url" || source.last_version.trim() == version)
    {
        dir = PathBuf::from(source.last_bundle_dir.trim());
    }
    if !dir.is_dir() {
        return Err(format!("版本 {version} 尚未下载到本机"));
    }

    let profile = crate::commands::profile::deployment_profile();
    let (bundle_version, commit) = validate_layout(&dir, &profile.backend_artifact)?;
    sources[index].last_version = if bundle_version.is_empty() {
        version.to_string()
    } else {
        bundle_version.clone()
    };
    sources[index].last_bundle_dir = dir.to_string_lossy().to_string();
    store_sources(&app, &sources)?;

    Ok(ArtifactBundle {
        dir: dir.to_string_lossy().to_string(),
        version: bundle_version,
        commit,
        md5: String::new(),
        size_bytes: 0,
        files: count_files(&dir),
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
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
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
}
