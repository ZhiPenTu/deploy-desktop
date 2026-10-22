//! 制品中心：制品源、访问令牌、云效版本、交付包下载与本机缓存。
//!
//! 命令层只做参数校验和调用编排；同目录子模块分工如下：
//! `store` 管配置与凭据，`yunxiao` 管云端字段映射，`cache` 管本机版本缓存，
//! `download` 管下载、解包与清理。命令名、参数与返回结构保持不变。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::{ipc::Channel, AppHandle};

mod cache;
mod download;
mod model;
mod store;
mod yunxiao;

use cache::*;
use download::*;
use model::*;
use store::*;
use yunxiao::*;

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
        created_at: existing
            .map(|index| tokens[index].created_at)
            .unwrap_or(now),
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
    let last_version = existing
        .map(|idx| sources[idx].last_version.clone())
        .unwrap_or_default();
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
        version: if kind == "url" {
            source.version.trim().to_string()
        } else {
            String::new()
        },
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

#[tauri::command]
pub async fn list_artifact_releases(
    app: AppHandle,
    id: String,
    page: Option<u32>,
) -> Result<ArtifactReleasePage, String> {
    tauri::async_runtime::spawn_blocking(move || list_artifact_releases_blocking(app, id, page))
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

#[tauri::command]
pub async fn prepare_local_artifact_bundle(
    app: AppHandle,
    path: String,
) -> Result<ArtifactBundle, String> {
    if path.trim().is_empty() {
        return Err("请先选择交付包文件或目录".to_string());
    }
    let cache_root = config_dir(&app)?.join("bundles/local");
    tauri::async_runtime::spawn_blocking(move || {
        prepare_local_bundle(Path::new(path.trim()), &cache_root)
    })
    .await
    .map_err(|e| format!("制品准备失败，请重试：{e}"))?
}

/// 删除某个制品源在本机缓存的全部交付包。
///
/// 只允许删除应用配置目录下 bundles/<id> 的内容；删除后清空索引里的
/// lastVersion / lastBundleDir，界面上该行不再显示"使用已下载"。
#[tauri::command]
pub async fn delete_local_bundle(app: AppHandle, id: String) -> Result<LocalBundleRemoval, String> {
    crate::commands::blocking::run(move || delete_local_bundle_blocking(app, id)).await
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
pub async fn open_external_url(url: String) -> Result<(), String> {
    crate::commands::blocking::run(move || open_external_url_blocking(url)).await
}

pub(crate) fn open_external_url_blocking(url: String) -> Result<(), String> {
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
    use serde_json::{json, Value};

    /// 按目录实际内容生成 v2 bundle.json，避免测试手写文件摘要。
    fn write_v2_bundle(dir: &Path, version: &str) {
        let mut files: Vec<serde_json::Value> = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            let Ok(entries) = fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
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
            left["path"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["path"].as_str().unwrap_or_default())
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
        fs::write(
            dir.join("bundle.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn local_artifact_archive_is_prepared_without_overwriting_source() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("local-artifact-{}-{stamp}", std::process::id()));
        let source = root.join("source");
        fs::create_dir_all(source.join("app/frontend-dist")).unwrap();
        fs::create_dir_all(source.join("deploy")).unwrap();
        fs::write(source.join("app/server.jar"), "artifact").unwrap();
        fs::write(source.join("app/frontend-dist/index.html"), "frontend").unwrap();
        fs::write(source.join("deploy/docker-compose.yml"), "services: {}\n").unwrap();
        write_v2_bundle(&source, "2");
        let archive = root.join("bundle.tgz");
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .arg(".")
            .status()
            .unwrap()
            .success());
        let prepared = prepare_local_bundle(&archive, &root.join("cache")).unwrap();
        assert_eq!(prepared.version, "2");
        assert_eq!(prepared.files, 3);
        assert!(Path::new(&prepared.dir)
            .join("deploy/docker-compose.yml")
            .is_file());
        assert_eq!(
            fs::read_to_string(source.join("app/server.jar")).unwrap(),
            "artifact"
        );
        let cache_count = fs::read_dir(root.join("cache")).unwrap().count();
        fs::remove_file(source.join("bundle.json")).unwrap();
        let invalid = root.join("invalid.tgz");
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&invalid)
            .arg("-C")
            .arg(&source)
            .arg(".")
            .status()
            .unwrap()
            .success());
        assert!(prepare_local_bundle(&invalid, &root.join("cache")).is_err());
        assert_eq!(
            fs::read_dir(root.join("cache")).unwrap().count(),
            cache_count
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_bundle_is_rejected_with_regeneration_hint() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("legacy-artifact-{}-{stamp}", std::process::id()));
        fs::create_dir_all(root.join("deploy")).unwrap();
        fs::write(root.join("deploy/docker-compose.yml"), "services: {}\n").unwrap();
        fs::write(
            root.join("bundle.json"),
            r#"{"version":"7","mode":"file","components":["backend"]}"#,
        )
        .unwrap();
        assert!(prepare_local_bundle(&root, &root.join("cache")).is_err());
        assert!(
            !root.join("cache").exists()
                || fs::read_dir(root.join("cache")).unwrap().next().is_none()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_artifact_rejects_links_before_reading_package_files() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("local-artifact-link-{stamp}"));
        fs::create_dir_all(root.join("source")).unwrap();
        fs::write(root.join("outside.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(root.join("outside.txt"), root.join("source/link")).unwrap();
        assert!(prepare_local_bundle(&root.join("source"), &root.join("cache")).is_err());
        let archive = root.join("bundle.tgz");
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(root.join("source"))
            .arg(".")
            .status()
            .unwrap()
            .success());
        assert!(prepare_local_bundle(&archive, &root.join("cache")).is_err());
        assert_eq!(
            fs::read_to_string(root.join("outside.txt")).unwrap(),
            "outside"
        );
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
        assert_eq!(
            json_str(&artifact, "md5"),
            "9de586e2c53cf541d46f118f31007f6c"
        );
        assert_eq!(find_key_i64(&detail, "startTime"), 1792405759000);
        assert_eq!(
            find_key_string(&detail, "CI_COMMIT_ID").as_deref(),
            Some("952e9dc")
        );
        assert!(validate_download_url(
            json_str(&artifact, "downloadUrl"),
            &source("yunxiaoFlow", "")
        )
        .is_ok());
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
        assert!(release_from_flow_detail(&item, 8, &detail)
            .unwrap()
            .is_none());
        detail["artifacts"][0]["artifact"] = "deploy-bundle.tgz".into();
        let release = release_from_flow_detail(&item, 8, &detail)
            .unwrap()
            .unwrap();
        assert_eq!(release.version, "8-a1b2c3d");
        detail["artifacts"][0]["version"] = 7.into();
        assert_eq!(
            release_from_flow_detail(&item, 7, &detail)
                .unwrap()
                .unwrap()
                .version,
            "7"
        );
        detail["artifacts"][0]["version"] = Value::Null;
        assert!(release_from_flow_detail(&item, 8, &detail)
            .unwrap_err()
            .contains("缺少版本号"));
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
        let root =
            std::env::temp_dir().join(format!("deploy-version-check-{}", std::process::id()));
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
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&fixture)
            .args(["app", "deploy", "bundle.json"])
            .status()
            .unwrap();
        assert!(status.success());
        let error = install_artifact_archive(&archive, &cached, "8-a1b2c3d").unwrap_err();
        assert!(error.contains("版本不匹配"));
        assert_eq!(
            fs::read_to_string(cached.join("previous.txt")).unwrap(),
            "preserve"
        );

        let mut item = source("yunxiaoFlow", "");
        item.id = "src-test".to_string();
        let release = ArtifactRelease {
            version: "8-a1b2c3d".to_string(),
            ..Default::default()
        };
        assert!(cached_bundle_dir(&root, &item, &release).is_none());
        write_v2_bundle(&fixture, "8-a1b2c3d");
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&fixture)
            .args(["app", "deploy", "bundle.json"])
            .status()
            .unwrap()
            .success());
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
        let root =
            std::env::temp_dir().join(format!("deploy-desktop-bundles-{}", std::process::id()));
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
        let dir =
            std::env::temp_dir().join(format!("deploy-desktop-tokens-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let id = format!("src-test-{}", std::process::id());

        secret_set(&dir, &id, "token-1").unwrap();
        assert_eq!(secret_get(&dir, &id).unwrap().as_deref(), Some("token-1"));
        assert!(secrets_file(&dir).is_file());

        secret_delete(&dir, &id).unwrap();
        let mut secrets = read_secrets(&dir).unwrap();
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
    fn corrupt_token_library_is_backed_up_instead_of_overwritten() {
        let root = cache_root("corrupt-token-library");
        fs::write(token_library_path(&root), "{\"tokens\": [").unwrap();

        let error = read_token_library(&root).unwrap_err();
        assert!(error.contains("已损坏"), "unexpected message: {error}");
        assert!(!token_library_path(&root).exists());

        let backup = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().contains(".corrupt-"))
            .expect("损坏的令牌列表应保留备份");
        assert_eq!(fs::read_to_string(backup).unwrap(), "{\"tokens\": [");

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
        let dir =
            std::env::temp_dir().join(format!("deploy-desktop-token-perm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        secret_set(&dir, "src-perm", "token").unwrap();
        let mode = fs::metadata(secrets_file(&dir))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
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
                    download_url: "https://example.aliyuncs.com/bundle?signature=secret"
                        .to_string(),
                    ..Default::default()
                }],
                1,
                true,
            ),
        );
        let local = root.join("bundles").join(&source.id).join("8");
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
            cached
                .items
                .iter()
                .map(|item| item.version.as_str())
                .collect::<Vec<_>>(),
            vec!["8", "7"]
        );
        assert!(!cached.has_more);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_bundle_build_times_with_timezone_offsets() {
        assert_eq!(
            parse_rfc3339_millis("2026-10-19T00:00:00Z"),
            Some(1_792_368_000_000)
        );
        assert_eq!(
            parse_rfc3339_millis("2026-10-19T08:00:00+08:00"),
            Some(1_792_368_000_000)
        );
        assert_eq!(parse_rfc3339_millis("not-a-time"), None);
    }
}
