//! 云效 Flow 的内容读取与字段映射。
//!
//! 云效返回的 JSON 字段随流水线类型变化，这里只做「按名字取第一个匹配值」的宽松解析，
//! 解析失败一律给出可操作的中文提示，不把原始响应直接抛给界面。

use serde_json::Value;
use std::time::Duration;

use super::*;

pub(super) const FLOW_API: &str = "https://openapi-rdc.aliyuncs.com/oapi/v1/flow/organizations";
pub(super) const FLOW_WEB: &str = "https://flow.aliyun.com/pipelines";
pub(super) const FLOW_RUN_PAGE_SIZE: u32 = 20;

pub(super) fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

pub(super) fn http_get_json(token: &str, url: &str) -> Result<Value, String> {
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

pub(super) fn collect_artifacts(value: &Value, out: &mut Vec<Value>) {
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

pub(super) fn key_matches(candidate: &str, key: &str, dotted: &str) -> bool {
    candidate == key || candidate.ends_with(dotted)
}

pub(super) fn find_key_string(value: &Value, key: &str) -> Option<String> {
    let dotted = format!(".{key}");
    find_key_string_inner(value, key, &dotted)
}

pub(super) fn find_key_string_inner(value: &Value, key: &str, dotted: &str) -> Option<String> {
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

pub(super) fn find_key_i64(value: &Value, key: &str) -> i64 {
    let dotted = format!(".{key}");
    find_key_i64_inner(value, key, &dotted)
}

pub(super) fn find_key_i64_inner(value: &Value, key: &str, dotted: &str) -> i64 {
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

pub(super) fn pick_artifact(value: &Value, artifact_name: &str) -> Option<Value> {
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

pub(super) fn json_i64(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}

pub(super) fn json_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

pub(super) fn json_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub(super) fn safe_version_key(version: &str) -> String {
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

pub(super) fn validate_artifact_version(version: &str) -> Result<&str, String> {
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

pub(super) fn url_release(source: &ArtifactSource) -> Result<ArtifactRelease, String> {
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

pub(super) fn release_from_flow_detail(
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
    let version = artifact
        .get("version")
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_string)
                .or_else(|| value.as_u64().map(|number| number.to_string()))
        })
        .unwrap_or_default();
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

pub(super) fn flow_run_items(
    token: &str,
    source: &ArtifactSource,
    page: u32,
) -> Result<Vec<Value>, String> {
    let url = format!(
        "{FLOW_API}/{}/pipelines/{}/runs?page={}&perPage={}",
        source.org_id,
        source.pipeline_id,
        page.max(1),
        FLOW_RUN_PAGE_SIZE
    );
    let runs = http_get_json(token, &url)?;
    runs.as_array()
        .cloned()
        .ok_or_else(|| "云效运行列表格式异常".to_string())
}

pub(super) fn load_flow_release_page(
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

pub(super) fn load_artifact_release_page(
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

pub(super) fn list_artifact_releases_blocking(
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
