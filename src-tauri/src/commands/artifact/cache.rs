//! 本机版本缓存：记录最近一次云端版本列表，并识别已下载的交付包。
//!
//! 缓存只用于加速首屏与断网展示；下载地址可能带临时签名，不写入本机文件。

use serde_json::Value;
use std::path::{Path, PathBuf};

use super::*;

pub(super) fn cached_bundle_dir(
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
    if last_dir.is_empty() || source.last_version.trim() != release.version.trim() {
        return None;
    }
    let last_dir = PathBuf::from(last_dir);
    bundle_has_version(&last_dir, &release.version).then_some(last_dir)
}

pub(super) fn bundle_has_version(dir: &Path, version: &str) -> bool {
    fs::read_to_string(dir.join("bundle.json"))
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .is_some_and(|meta| {
            meta.get("metadata")
                .map(|metadata| json_str(metadata, "version").trim() == version.trim())
                .unwrap_or(false)
        })
}

pub(super) fn release_cache_path(config_root: &Path) -> PathBuf {
    config_root.join("artifact-releases-cache.json")
}

pub(super) fn release_cache(config_root: &Path) -> ReleaseCacheFile {
    fs::read_to_string(release_cache_path(config_root))
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

pub(super) fn source_signature(source: &ArtifactSource) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        source.kind, source.org_id, source.pipeline_id, source.artifact_name, source.download_url
    )
}

pub(super) fn release_cache_key(item: &ArtifactRelease) -> String {
    format!("{}|{}|{}", item.run_id, item.artifact_name, item.version)
}

/// 记录最近一次成功读取的云端版本列表，供下次打开页面或断网时先展示。
/// 下载地址可能带有临时签名，只留在当前会话，不写入本地缓存文件。
pub(super) fn store_release_cache(
    config_root: &Path,
    source: &ArtifactSource,
    page: &ArtifactReleasePage,
) {
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
            if !entry
                .items
                .iter()
                .any(|known| release_cache_key(known) == release_cache_key(&item))
            {
                entry.items.push(item);
            }
        }
    }
    entry.signature = signature;
    entry.has_more = page.has_more;
    entry.cached_at = now_millis().min(i64::MAX as u128) as i64;
    let _ = crate::core::json_store::save(
        &release_cache_path(config_root),
        &file,
        "版本缓存",
        crate::core::atomic_file::FilePermission::Default,
    );
}

pub(super) fn remove_release_cache(config_root: &Path, id: &str) {
    let mut file = release_cache(config_root);
    if file.sources.remove(id).is_none() {
        return;
    }
    let _ = crate::core::json_store::save(
        &release_cache_path(config_root),
        &file,
        "版本缓存",
        crate::core::atomic_file::FilePermission::Default,
    );
}

pub(super) fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
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

pub(super) fn parse_rfc3339_millis(text: &str) -> Option<i64> {
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

pub(super) fn bundle_meta_size(meta: &Value) -> u64 {
    meta.get("files")
        .and_then(Value::as_array)
        .map(|files| files.iter().map(|item| json_u64(item, "size")).sum())
        .unwrap_or_default()
}

pub(super) fn release_from_local_bundle(
    source: &ArtifactSource,
    dir: &Path,
) -> Option<ArtifactRelease> {
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

pub(super) fn local_bundle_dirs(config_root: &Path, source: &ArtifactSource) -> Vec<PathBuf> {
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

pub(super) fn cached_release_page(
    config_root: &Path,
    source: &ArtifactSource,
) -> ArtifactReleasePage {
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

pub(super) fn attach_local_bundle(
    config_root: &Path,
    source: &ArtifactSource,
    release: &mut ArtifactRelease,
) {
    if let Some(dir) = cached_bundle_dir(config_root, source, release) {
        release.local_dir = dir.to_string_lossy().to_string();
    }
}
