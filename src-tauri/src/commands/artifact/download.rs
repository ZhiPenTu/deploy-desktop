//! 交付包下载、解包、安装与本地缓存清理。
//!
//! 下载内容先落临时文件，校验通过后再解包到配置目录；解包时拒绝绝对路径、
//! 父目录跳转与符号链接，避免压缩包把文件写到目标目录之外。

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use tauri::{ipc::Channel, AppHandle};

use super::*;
use crate::core::backup;

pub(super) const MAX_ARTIFACT_BYTES: u64 = 5 * 1024 * 1024 * 1024;
pub(super) fn fetch_artifact_release_blocking(
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

pub(super) fn validate_download_url(url: &str, source: &ArtifactSource) -> Result<(), String> {
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

pub(super) fn download_to_file(
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

pub(super) fn safe_extract(archive: &Path, dest: &Path) -> Result<usize, String> {
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

pub(super) fn is_safe_entry(name: &str) -> bool {
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

pub(super) fn validate_layout(dir: &Path) -> Result<(String, String), String> {
    if !dir.join("bundle.json").is_file() {
        return Err("构建物缺少 bundle.json，可能不是 deploy-desktop 交付包".to_string());
    }
    // 结构、文件清单与摘要都在描述符里校验，客户端不再假设固定目录结构。
    let bundle = crate::commands::bundle::Bundle::load(dir)?;
    bundle.verify_payload()?;
    let version = validate_artifact_version(bundle.version())?.to_string();
    Ok((version, bundle.metadata().commit_short.clone()))
}

pub(super) fn install_artifact_archive(
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
        let previous = staged.with_extension("previous");
        let had_previous = backup::swap_staged(&staged, dest, &previous).map_err(|failure| {
            match failure.stage {
                backup::SwapStage::MoveAside => {
                    format!("保留已有交付包失败：{}", failure.error)
                }
                backup::SwapStage::Install => match failure.rollback {
                    Ok(()) => format!("保存交付包失败：{}", failure.error),
                    Err(restore_error) => format!(
                        "保存交付包失败：{}；恢复原交付包失败：{}",
                        failure.error, restore_error
                    ),
                },
            }
        })?;
        let _ = backup::discard_previous(&previous, had_previous);
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

pub(super) fn download_artifact_blocking(
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

pub(super) fn count_files(dir: &Path) -> usize {
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
pub(super) fn payload_file_count(dir: &Path) -> usize {
    match crate::commands::bundle::Bundle::load(dir) {
        Ok(bundle) => bundle.payload_file_count(),
        Err(_) => count_files(dir),
    }
}

// 本地目录与压缩包共用同一结构检查；不跟随符号链接，避免部署时访问包外文件。
pub(super) fn inspect_local_tree(dir: &Path, depth: usize) -> Result<(usize, u64), String> {
    if depth > 128 {
        return Err("交付包目录层级过深，请检查后重试".to_string());
    }
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in fs::read_dir(dir).map_err(|e| format!("读取交付包目录失败：{e}"))? {
        let entry = entry.map_err(|e| format!("读取交付包文件失败：{e}"))?;
        let meta =
            fs::symlink_metadata(entry.path()).map_err(|e| format!("检查交付包文件失败：{e}"))?;
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

pub(super) fn prepare_local_bundle(
    path: &Path,
    cache_root: &Path,
) -> Result<ArtifactBundle, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("读取所选制品失败：{e}"))?;
    if meta.file_type().is_symlink() {
        return Err("请选择交付包原文件或目录，不能使用符号链接".to_string());
    }
    let source = fs::canonicalize(path).map_err(|e| format!("读取所选制品失败：{e}"))?;
    let extracted = if meta.is_dir() {
        None
    } else if meta.is_file() {
        let name = source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if !name.ends_with(".tgz") && !name.ends_with(".tar.gz") {
            return Err("请选择 .tgz、.tar.gz 交付包，或已解压的交付包目录".to_string());
        }
        if meta.len() > MAX_ARTIFACT_BYTES {
            return Err("交付包超过 5 GiB，请检查所选文件".to_string());
        }
        fs::create_dir_all(cache_root).map_err(|e| format!("创建交付包目录失败：{e}"))?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "无法生成交付包目录，请检查系统时间".to_string())?
            .as_nanos();
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
            dir: dir.to_string_lossy().to_string(),
            version,
            commit,
            md5: String::new(),
            size_bytes: bytes,
            files,
        })
    })();
    if result.is_err() {
        if let Some(dest) = extracted {
            let _ = fs::remove_dir_all(dest);
        }
    }
    result
}

pub(super) fn dir_size_bytes(dir: &Path) -> u64 {
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

pub(super) fn delete_local_bundle_blocking(
    app: AppHandle,
    id: String,
) -> Result<LocalBundleRemoval, String> {
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
    crate::commands::projects::clear_bundle_dir(&config_root.join("bundles").join(&id));

    Ok(removal)
}

pub(super) fn remove_bundle_tree(
    config_root: &Path,
    id: &str,
) -> Result<LocalBundleRemoval, String> {
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
