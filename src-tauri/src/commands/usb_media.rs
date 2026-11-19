//! USB 中转：把本机交付包与离线镜像包写入 USB 介质，或从介质读回用于部署更新。
//!
//! 介质目录固定在所选目录下的 `部署舱介质/`，结构如下：
//!
//! ```text
//! 部署舱介质/
//!   media.json                 介质清单（条目、大小与摘要）
//!   artifacts/<名称>-<版本>.tgz 交付包
//!   images/<离线包文件名>        镜像离线包
//! ```
//!
//! 写入时先落同目录临时文件再改名，摘要与大小在复制过程中计算并写入清单；
//! 读取时只接受清单内的相对路径，拒绝目录跳转、绝对路径与符号链接。
//! 介质内容只用于跨机转运，不保存凭据或制品源配置。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use tauri::ipc::Channel;
use tauri::AppHandle;

use crate::core::atomic_file;

const MEDIA_ROOT_NAME: &str = "部署舱介质";
const MANIFEST_FILE: &str = "media.json";
const ARTIFACT_DIR: &str = "artifacts";
const IMAGE_DIR: &str = "images";
const SCHEMA_VERSION: u32 = 1;
const COPY_BUFFER: usize = 1024 * 1024;
const PROGRESS_INTERVAL_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct MediaManifest {
    schema_version: u32,
    updated_at: String,
    entries: Vec<MediaEntry>,
}

impl Default for MediaManifest {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            updated_at: String::new(),
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaEntry {
    pub id: String,
    pub kind: String,
    pub file: String,
    pub label: String,
    pub version: String,
    pub product: String,
    pub display_name: String,
    pub images: Vec<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub added_at: String,
}

impl Default for MediaEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: String::new(),
            file: String::new(),
            label: String::new(),
            version: String::new(),
            product: String::new(),
            display_name: String::new(),
            images: Vec::new(),
            size_bytes: 0,
            sha256: String::new(),
            added_at: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbEntryView {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub version: String,
    pub product: String,
    pub display_name: String,
    pub images: Vec<String>,
    pub file: String,
    pub file_path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub added_at: String,
    pub present: bool,
    pub issue: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbMediaScan {
    pub selected_dir: String,
    pub root_dir: String,
    pub exists: bool,
    pub writable: bool,
    pub entries: Vec<UsbEntryView>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBundleSummary {
    pub dir: String,
    pub label: String,
    pub product: String,
    pub version: String,
    pub commit: String,
    pub source_name: String,
    pub size_bytes: u64,
    pub modified_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbItemResult {
    pub kind: String,
    pub label: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbExportResult {
    pub written: usize,
    pub failed: usize,
    pub items: Vec<UsbItemResult>,
    pub scan: UsbMediaScan,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbVerification {
    pub valid: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbImageImport {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbProgress {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub label: String,
    pub percent: usize,
    pub message: String,
    pub finished: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbImageInput {
    pub path: String,
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct BundleMetadata {
    product: String,
    display_name: String,
    version: String,
    commit: String,
    commit_short: String,
}

struct MediaRoots {
    selected: PathBuf,
    root: PathBuf,
    artifacts: PathBuf,
    images: PathBuf,
}

#[tauri::command]
pub async fn scan_usb_media(media_dir: String) -> Result<UsbMediaScan, String> {
    crate::commands::blocking::run(move || scan_usb_media_blocking(&media_dir)).await
}

#[tauri::command]
pub async fn list_local_artifact_bundles(
    app: AppHandle,
) -> Result<Vec<LocalBundleSummary>, String> {
    crate::commands::blocking::run(move || {
        let config_root = crate::core::config::app_config_dir(&app)?;
        let source_names = super::artifact::artifact_source_names(&app);
        list_local_bundles_blocking(&config_root, &source_names)
    })
    .await
}

#[tauri::command]
pub async fn verify_usb_entry(media_dir: String, entry_id: String) -> Result<UsbVerification, String> {
    crate::commands::blocking::run(move || verify_usb_entry_blocking(&media_dir, &entry_id)).await
}

#[tauri::command]
pub async fn export_usb_media(
    app: AppHandle,
    media_dir: String,
    artifact_dirs: Vec<String>,
    image_files: Vec<UsbImageInput>,
    on_event: Channel<UsbProgress>,
) -> Result<UsbExportResult, String> {
    let config_root = crate::core::config::app_config_dir(&app)?;
    crate::commands::blocking::run(move || {
        export_usb_media_blocking(
            &config_root,
            &media_dir,
            artifact_dirs,
            image_files,
            &on_event,
        )
    })
    .await
}

#[tauri::command]
pub async fn import_usb_image(
    app: AppHandle,
    media_dir: String,
    entry_id: String,
    on_event: Channel<UsbProgress>,
) -> Result<UsbImageImport, String> {
    crate::commands::blocking::run(move || {
        import_usb_image_blocking(&app, &media_dir, &entry_id, &on_event)
    })
    .await
}

fn scan_usb_media_blocking(media_dir: &str) -> Result<UsbMediaScan, String> {
    let roots = resolve_media(media_dir)?;
    let exists = roots.root.is_dir();
    let writable = probe_writable(if exists { &roots.root } else { &roots.selected });
    let entries = if exists {
        load_manifest(&roots.root)?
            .map(|manifest| manifest.entries)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut views = Vec::with_capacity(entries.len());
    let mut total_bytes = 0u64;
    for entry in entries {
        let view = entry_view(&roots.root, &entry);
        total_bytes += view.size_bytes;
        views.push(view);
    }
    views.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| right.added_at.cmp(&left.added_at))
            .then_with(|| left.label.cmp(&right.label))
    });

    Ok(UsbMediaScan {
        selected_dir: roots.selected.display().to_string(),
        root_dir: roots.root.display().to_string(),
        exists,
        writable,
        entries: views,
        total_bytes,
    })
}

fn export_usb_media_blocking(
    config_root: &Path,
    media_dir: &str,
    artifact_dirs: Vec<String>,
    image_files: Vec<UsbImageInput>,
    on_event: &Channel<UsbProgress>,
) -> Result<UsbExportResult, String> {
    if artifact_dirs.is_empty() && image_files.is_empty() {
        return Err("请选择要写入 USB 的交付包或镜像包".to_string());
    }

    let roots = resolve_media_for_write(media_dir, config_root)?;
    fs::create_dir_all(&roots.artifacts)
        .map_err(|error| format!("创建介质目录失败：{error}"))?;
    fs::create_dir_all(&roots.images).map_err(|error| format!("创建介质目录失败：{error}"))?;

    let mut manifest = load_manifest(&roots.root)?.unwrap_or_default();
    let artifact_dirs = dedupe(artifact_dirs);
    let artifact_total = artifact_dirs.len();
    let total = artifact_total + image_files.len();
    let mut items = Vec::with_capacity(total);
    let mut written = 0usize;
    let mut failed = 0usize;
    let mut used_names: Vec<String> = Vec::new();
    let stamp = timestamp_text();

    for (index, raw_dir) in artifact_dirs.iter().enumerate() {
        let current = index + 1;
        let source = PathBuf::from(raw_dir);
        emit(
            on_event,
            "artifact",
            current,
            total,
            raw_dir,
            progress_percent(index, total),
            "正在写入交付包",
            false,
        );
        match write_artifact(
            &roots,
            &source,
            &mut used_names,
            &stamp,
            |percent| {
                emit(
                    on_event,
                    "artifact",
                    current,
                    total,
                    raw_dir,
                    progress_percent(index, total) + percent / total.max(1),
                    "正在写入交付包",
                    false,
                );
            },
        ) {
            Ok(mut entry) => {
                let label = entry.label.clone();
                manifest
                    .entries
                    .retain(|item| item.id != entry.id && item.file != entry.file);
                entry.added_at = stamp.clone();
                manifest.entries.push(entry);
                written += 1;
                items.push(UsbItemResult {
                    kind: String::from("artifact"),
                    label,
                    success: true,
                    message: String::from("已写入"),
                });
            }
            Err(error) => {
                failed += 1;
                items.push(UsbItemResult {
                    kind: String::from("artifact"),
                    label: label_for_path(&source),
                    success: false,
                    message: error,
                });
            }
        }
    }

    for (index, input) in image_files.into_iter().enumerate() {
        let current = artifact_total + index + 1;
        let source = PathBuf::from(input.path.trim());
        let label = label_for_path(&source);
        emit(
            on_event,
            "image",
            current,
            total,
            &label,
            progress_percent(artifact_total + index, total),
            "正在写入镜像包",
            false,
        );
        match write_image(
            &roots,
            &source,
            &input.images,
            &mut used_names,
            &stamp,
            |percent| {
                let base = progress_percent(artifact_total + index, total);
                let span = if total == 0 { 100 } else { 100 / total };
                emit(
                    on_event,
                    "image",
                    current,
                    total,
                    &label,
                    base + percent.saturating_mul(span) / 100,
                    "正在写入镜像包",
                    false,
                );
            },
        ) {
            Ok(mut entry) => {
                let entry_label = entry.label.clone();
                manifest
                    .entries
                    .retain(|item| item.id != entry.id && item.file != entry.file);
                entry.added_at = stamp.clone();
                manifest.entries.push(entry);
                written += 1;
                items.push(UsbItemResult {
                    kind: String::from("image"),
                    label: entry_label,
                    success: true,
                    message: String::from("已写入"),
                });
            }
            Err(error) => {
                failed += 1;
                items.push(UsbItemResult {
                    kind: String::from("image"),
                    label,
                    success: false,
                    message: error,
                });
            }
        }
    }

    if written > 0 {
        manifest.schema_version = SCHEMA_VERSION;
        manifest.updated_at = stamp;
        save_manifest(&roots.root, &manifest)?;
    }

    emit(
        on_event,
        "done",
        total,
        total,
        "",
        100,
        "写入完成",
        true,
    );

    let scan = scan_usb_media_blocking(media_dir)?;
    Ok(UsbExportResult {
        written,
        failed,
        items,
        scan,
    })
}

fn import_usb_image_blocking(
    app: &AppHandle,
    media_dir: &str,
    entry_id: &str,
    on_event: &Channel<UsbProgress>,
) -> Result<UsbImageImport, String> {
    let roots = resolve_media(media_dir)?;
    let manifest = load_manifest(&roots.root)?
        .ok_or_else(|| "介质中没有可读取的内容，请先写入 USB".to_string())?;
    let id = entry_id.trim();
    let entry = manifest
        .entries
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "该内容已不在介质清单中，请刷新后重试".to_string())?;
    if entry.kind != "image" {
        return Err("该条目不是镜像包".to_string());
    }

    let source = resolve_entry_file(&roots.root, entry)?;
    let file_name = entry_file_name(entry)?;
    if !is_image_file(&file_name) {
        return Err("介质中的镜像包文件名无效，请重新写入".to_string());
    }

    emit(
        on_event,
        "verify",
        1,
        1,
        &entry.label,
        0,
        "正在校验镜像包",
        false,
    );
    let size = entry.size_bytes;
    let digest = hash_file_with_progress(&source, size, |percent| {
        emit(
            on_event,
            "verify",
            1,
            1,
            &entry.label,
            percent / 2,
            "正在校验镜像包",
            false,
        );
    })?;
    if digest != entry.sha256 {
        return Err("镜像包校验未通过，文件可能已损坏，请重新写入 USB".to_string());
    }

    let offline_dir = super::image_library::effective_offline_dir(app);
    fs::create_dir_all(&offline_dir).map_err(|error| format!("创建离线包目录失败：{error}"))?;
    let target = offline_dir.join(&file_name);
    let mut message = String::from("已复制到离线包目录");
    if target.is_file() {
        let existing = super::bundle::sha256_path(&target)?;
        if existing == entry.sha256 {
            message = String::from("离线包目录中已有相同的镜像包");
            super::image_library::record_offline_package(&offline_dir, &file_name, &entry.images)?;
            emit(
                on_event,
                "done",
                1,
                1,
                &entry.label,
                100,
                &message,
                true,
            );
            return Ok(UsbImageImport {
                path: target.display().to_string(),
                message,
            });
        }
        return Err(format!(
            "离线包目录中已有同名但内容不同的文件：{file_name}，请先在「镜像与离线包」中处理"
        ));
    }

    emit(
        on_event,
        "copy",
        1,
        1,
        &entry.label,
        50,
        "正在复制镜像包",
        false,
    );
    let (written_digest, written_size) = copy_file_hashed(&source, &target, |percent| {
        emit(
            on_event,
            "copy",
            1,
            1,
            &entry.label,
            50 + percent / 2,
            "正在复制镜像包",
            false,
        );
    })?;
    if written_digest != entry.sha256 || written_size != entry.size_bytes {
        let _ = fs::remove_file(&target);
        return Err("镜像包复制后校验未通过，请重试".to_string());
    }
    super::image_library::record_offline_package(&offline_dir, &file_name, &entry.images)?;

    emit(
        on_event,
        "done",
        1,
        1,
        &entry.label,
        100,
        &message,
        true,
    );
    Ok(UsbImageImport {
        path: target.display().to_string(),
        message,
    })
}

fn verify_usb_entry_blocking(media_dir: &str, entry_id: &str) -> Result<UsbVerification, String> {
    let roots = resolve_media(media_dir)?;
    let manifest = load_manifest(&roots.root)?
        .ok_or_else(|| "介质中没有可读取的内容，请先写入 USB".to_string())?;
    let entry = manifest
        .entries
        .iter()
        .find(|item| item.id == entry_id.trim())
        .ok_or_else(|| "该内容已不在介质清单中，请刷新后重试".to_string())?;
    let source = resolve_entry_file(&roots.root, entry)?;
    let size = fs::metadata(&source)
        .map_err(|error| format!("读取介质文件失败：{error}"))?
        .len();
    if size != entry.size_bytes {
        return Ok(UsbVerification {
            valid: false,
            message: String::from("文件大小与介质清单不一致，可能未写入完整"),
        });
    }
    let digest = super::bundle::sha256_path(&source)?;
    if digest != entry.sha256 {
        return Ok(UsbVerification {
            valid: false,
            message: String::from("文件内容校验未通过，请重新写入 USB"),
        });
    }
    Ok(UsbVerification {
        valid: true,
        message: String::from("校验通过"),
    })
}

fn list_local_bundles_blocking(
    config_root: &Path,
    source_names: &[(String, String)],
) -> Result<Vec<LocalBundleSummary>, String> {
    let bundles_root = config_root.join("bundles");
    if !bundles_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut dirs = Vec::new();
    collect_bundle_dirs(&bundles_root, 0, &mut dirs);
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for dir in dirs {
        let Ok(canonical) = fs::canonicalize(&dir) else {
            continue;
        };
        if !seen.insert(canonical) {
            continue;
        }
        let Some(meta) = read_bundle_metadata(&dir) else {
            continue;
        };
        let relative = dir.strip_prefix(&bundles_root).unwrap_or(&dir);
        let source_id = relative
            .components()
            .next()
            .and_then(|component| match component {
                Component::Normal(name) => name.to_str(),
                _ => None,
            })
            .unwrap_or_default();
        let source_name = source_names
            .iter()
            .find(|(id, _)| id == source_id)
            .map(|(_, name)| name.clone())
            .unwrap_or_default();
        let label = display_label(&meta);
        let commit = if meta.commit_short.trim().is_empty() {
            meta.commit.trim().to_string()
        } else {
            meta.commit_short.trim().to_string()
        };
        items.push(LocalBundleSummary {
            dir: dir.display().to_string(),
            label,
            product: meta.product.trim().to_string(),
            version: meta.version.trim().to_string(),
            commit,
            source_name,
            size_bytes: dir_size(&dir),
            modified_ms: modified_ms(&dir),
        });
    }
    items.sort_by(|left, right| right.modified_ms.cmp(&left.modified_ms));
    Ok(items)
}

fn collect_bundle_dirs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 4 {
        return;
    }
    if dir.join("bundle.json").is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with('.')
        {
            continue;
        }
        collect_bundle_dirs(&entry.path(), depth + 1, out);
    }
}

fn write_artifact(
    roots: &MediaRoots,
    source: &Path,
    used_names: &mut Vec<String>,
    stamp: &str,
    mut progress: impl FnMut(usize),
) -> Result<MediaEntry, String> {
    let raw = source.display().to_string();
    if raw.trim().is_empty() {
        return Err(String::from("交付包路径为空"));
    }
    let meta = fs::symlink_metadata(source)
        .map_err(|error| format!("无法读取所选交付包：{error}"))?;
    if meta.file_type().is_symlink() {
        return Err(String::from("不能使用符号链接作为交付包，请选择实际目录"));
    }
    if !meta.is_dir() {
        return Err(String::from("所选交付包不是目录，请重新选择"));
    }
    ensure_outside_media(source, &roots.root)?;

    let bundle_meta = read_bundle_metadata(source)
        .ok_or_else(|| String::from("交付包缺少可用的 bundle.json，请先确认本机交付包"))?;
    let version = bundle_meta.version.trim();
    if version.is_empty() {
        return Err(String::from("交付包未声明版本号，无法写入 USB"));
    }
    let label = display_label(&bundle_meta);
    let file_name = unique_name(
        used_names,
        &format!(
            "{}-{}.tgz",
            sanitize_name(&label, "交付包"),
            sanitize_name(version, "版本")
        ),
    );
    let target = roots.artifacts.join(&file_name);
    let relative = format!("{ARTIFACT_DIR}/{file_name}");
    let temporary = temporary_path(&target);

    let result = pack_bundle(source, &temporary).and_then(|(digest, size)| {
        replace_file(&temporary, &target)?;
        Ok((digest, size))
    });
    let (digest, size) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    progress(100);

    Ok(MediaEntry {
        id: entry_id("artifact", &digest),
        kind: String::from("artifact"),
        file: relative,
        label,
        version: version.to_string(),
        product: bundle_meta.product.trim().to_string(),
        display_name: bundle_meta.display_name.trim().to_string(),
        images: Vec::new(),
        size_bytes: size,
        sha256: digest,
        added_at: stamp.to_string(),
    })
}

fn write_image(
    roots: &MediaRoots,
    source: &Path,
    images: &[String],
    used_names: &mut Vec<String>,
    stamp: &str,
    mut progress: impl FnMut(usize),
) -> Result<MediaEntry, String> {
    let meta = fs::symlink_metadata(source)
        .map_err(|error| format!("无法读取所选镜像包：{error}"))?;
    if meta.file_type().is_symlink() {
        return Err(String::from("不能使用符号链接作为镜像包，请选择实际文件"));
    }
    if !meta.is_file() {
        return Err(String::from("所选镜像包不是文件，请重新选择"));
    }
    ensure_outside_media(source, &roots.root)?;
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| String::from("镜像包文件名无法识别，请重新导出"))?
        .to_string();
    if !is_image_file(&file_name) {
        return Err(String::from("只能写入 .tar、.tgz 或 .tar.gz 镜像包"));
    }
    let file_name = unique_name(used_names, &file_name);
    let target = roots.images.join(&file_name);
    let relative = format!("{IMAGE_DIR}/{file_name}");
    let (digest, size) = match copy_file_hashed(source, &target, &mut progress) {
        Ok(value) => value,
        Err(error) => return Err(error),
    };

    Ok(MediaEntry {
        id: entry_id("image", &digest),
        kind: String::from("image"),
        file: relative,
        label: file_name,
        version: String::new(),
        product: String::new(),
        display_name: String::new(),
        images: images.to_vec(),
        size_bytes: size,
        sha256: digest,
        added_at: stamp.to_string(),
    })
}

fn pack_bundle(source: &Path, temporary: &Path) -> Result<(String, u64), String> {
    let _ = fs::remove_file(temporary);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)
        .map_err(|error| format!("创建介质文件失败：{error}"))?;
    let sink = HashingWriter {
        inner: file,
        hasher: Sha256::new(),
        bytes: 0,
    };
    let encoder = flate2::write::GzEncoder::new(sink, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.follow_symlinks(false);
    append_tree(&mut builder, source, Path::new(""))?;
    let encoder = builder
        .into_inner()
        .map_err(|error| format!("打包交付包失败：{error}"))?;
    let sink = encoder
        .finish()
        .map_err(|error| format!("打包交付包失败：{error}"))?;
    sink.inner
        .sync_all()
        .map_err(|error| format!("写入介质失败：{error}"))?;
    let digest = sink.digest();
    Ok((digest, sink.bytes))
}

fn append_tree<W: Write>(
    builder: &mut tar::Builder<W>,
    root: &Path,
    relative: &Path,
) -> Result<(), String> {
    let current = if relative.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    let mut entries = fs::read_dir(&current)
        .map_err(|error| format!("读取交付包目录失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取交付包目录失败：{error}"))?;
    entries.sort_by(|left, right| left.file_name().cmp(&right.file_name()));

    for entry in entries {
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| String::from("交付包含有无法识别的文件名"))?;
        if name == ".DS_Store" {
            continue;
        }
        let child = if relative.as_os_str().is_empty() {
            PathBuf::from(&name)
        } else {
            relative.join(&name)
        };
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)
            .map_err(|error| format!("读取交付包文件失败：{error}"))?;
        if meta.file_type().is_symlink() {
            return Err(format!("交付包包含符号链接：{}", child.display()));
        }
        if meta.is_dir() {
            builder
                .append_path_with_name(&path, &child)
                .map_err(|error| format!("打包交付包失败：{error}"))?;
            append_tree(builder, root, &child)?;
        } else if meta.is_file() {
            builder
                .append_path_with_name(&path, &child)
                .map_err(|error| format!("打包交付包失败：{error}"))?;
        } else {
            return Err(format!("交付包包含无法打包的文件：{}", child.display()));
        }
    }
    Ok(())
}

fn copy_file_hashed(
    source: &Path,
    target: &Path,
    mut progress: impl FnMut(usize),
) -> Result<(String, u64), String> {
    let total = fs::metadata(source)
        .map_err(|error| format!("读取源文件失败：{error}"))?
        .len();
    let temporary = temporary_path(target);
    let _ = fs::remove_file(&temporary);
    let result = (|| {
        let mut input = fs::File::open(source)
            .map_err(|error| format!("读取源文件失败：{error}"))?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("创建介质文件失败：{error}"))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; COPY_BUFFER];
        let mut written = 0u64;
        let mut reported = 0u64;
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|error| format!("读取源文件失败：{error}"))?;
            if read == 0 {
                break;
            }
            hasher
                .update(&buffer[..read]);
            output
                .write_all(&buffer[..read])
                .map_err(|error| format!("写入介质失败：{error}"))?;
            written += read as u64;
            if written - reported >= PROGRESS_INTERVAL_BYTES || written == total {
                reported = written;
                progress(percent_of(written, total));
            }
        }
        output
            .sync_all()
            .map_err(|error| format!("写入介质失败：{error}"))?;
        drop(output);
        let digest = hex_digest(hasher.finalize());
        if written != total {
            return Err(String::from("源文件在复制过程中发生变化，请重试"));
        }
        replace_file(&temporary, target)?;
        Ok((digest, written))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn hash_file_with_progress(
    path: &Path,
    total: u64,
    mut progress: impl FnMut(usize),
) -> Result<String, String> {
    let mut input = fs::File::open(path).map_err(|error| format!("读取介质文件失败：{error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut read_total = 0u64;
    let mut reported = 0u64;
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("读取介质文件失败：{error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        read_total += read as u64;
        if read_total - reported >= PROGRESS_INTERVAL_BYTES || read_total == total {
            reported = read_total;
            progress(percent_of(read_total, total));
        }
    }
    Ok(hex_digest(hasher.finalize()))
}

fn resolve_media(media_dir: &str) -> Result<MediaRoots, String> {
    let selected = media_selection(media_dir)?;
    let root = media_root(&selected);
    Ok(MediaRoots {
        selected,
        artifacts: root.join(ARTIFACT_DIR),
        images: root.join(IMAGE_DIR),
        root,
    })
}

fn resolve_media_for_write(media_dir: &str, config_root: &Path) -> Result<MediaRoots, String> {
    let roots = resolve_media(media_dir)?;
    let canonical_config = fs::canonicalize(config_root)
        .map_err(|error| format!("读取应用配置目录失败：{error}"))?;
    let canonical_selected = fs::canonicalize(&roots.selected)
        .map_err(|error| format!("读取所选目录失败：{error}"))?;
    if canonical_selected.starts_with(&canonical_config) {
        return Err(String::from(
            "不能选择应用配置目录作为 USB 介质，请选择 U 盘或移动硬盘中的目录",
        ));
    }
    if !roots.writable_parent() {
        return Err(String::from("所选目录不可写入，请检查 U 盘是否已插入并解除只读"));
    }
    Ok(roots)
}

impl MediaRoots {
    fn writable_parent(&self) -> bool {
        if self.root.is_dir() {
            probe_writable(&self.root)
        } else {
            self.root
                .parent()
                .map(probe_writable)
                .unwrap_or(false)
        }
    }
}

fn media_selection(media_dir: &str) -> Result<PathBuf, String> {
    let raw = media_dir.trim();
    if raw.is_empty() {
        return Err("请先选择 USB 介质目录".to_string());
    }
    let selected = PathBuf::from(raw);
    if !selected.is_absolute() {
        return Err(String::from("请选择 USB 介质的完整路径"));
    }
    let meta = fs::symlink_metadata(&selected).map_err(|_| {
        String::from("无法访问所选目录，请确认 USB 已连接后重新选择")
    })?;
    if meta.file_type().is_symlink() {
        return Err(String::from("不能使用符号链接作为 USB 介质目录，请重新选择"));
    }
    if !meta.is_dir() {
        return Err(String::from("所选路径不是文件夹，请重新选择"));
    }
    Ok(selected)
}

fn media_root(selected: &Path) -> PathBuf {
    if selected
        .file_name()
        .and_then(|name| name.to_str())
        == Some(MEDIA_ROOT_NAME)
    {
        selected.to_path_buf()
    } else {
        selected.join(MEDIA_ROOT_NAME)
    }
}

fn probe_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(
        ".deploy-desktop-media-probe-{}",
        std::process::id()
    ));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn ensure_outside_media(source: &Path, root: &Path) -> Result<(), String> {
    let canonical_source = fs::canonicalize(source)
        .map_err(|error| format!("读取所选路径失败：{error}"))?;
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if canonical_source.starts_with(&canonical_root) {
        return Err(String::from("不能把介质内的文件再次写入介质，请选择本机文件"));
    }
    Ok(())
}

fn replace_file(temporary: &Path, target: &Path) -> Result<(), String> {
    if fs::rename(temporary, target).is_ok() {
        return Ok(());
    }
    if target.exists() {
        fs::remove_file(target).map_err(|error| format!("替换介质文件失败：{error}"))?;
        fs::rename(temporary, target).map_err(|error| format!("保存文件到介质失败：{error}"))?;
        return Ok(());
    }
    Err(String::from("保存文件到介质失败，请检查 USB 是否已满"))
}

fn entry_view(root: &Path, entry: &MediaEntry) -> UsbEntryView {
    let relative = safe_relative(&entry.file);
    let path = relative
        .as_ref()
        .map(|relative| root.join(relative))
        .unwrap_or_default();
    let mut present = false;
    let mut issue = if relative.is_none() {
        String::from("介质清单中的文件路径无效")
    } else {
        String::new()
    };
    if relative.is_some() {
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                present = true;
                if meta.len() != entry.size_bytes {
                    issue = String::from("文件大小与清单不一致");
                }
            }
            Ok(_) => {
                issue = String::from("介质中的条目不是普通文件");
            }
            Err(_) => {
                issue = String::from("介质中缺少该文件");
            }
        }
    }
    UsbEntryView {
        id: entry.id.clone(),
        kind: entry.kind.clone(),
        label: if entry.label.trim().is_empty() {
            entry.file.clone()
        } else {
            entry.label.clone()
        },
        version: entry.version.clone(),
        product: entry.product.clone(),
        display_name: entry.display_name.clone(),
        images: entry.images.clone(),
        file: entry.file.clone(),
        file_path: if present || relative.is_some() {
            path.display().to_string()
        } else {
            String::new()
        },
        size_bytes: entry.size_bytes,
        sha256: entry.sha256.clone(),
        added_at: entry.added_at.clone(),
        present: present && issue.is_empty(),
        issue,
    }
}

fn resolve_entry_file(root: &Path, entry: &MediaEntry) -> Result<PathBuf, String> {
    let relative = safe_relative(&entry.file)
        .ok_or_else(|| String::from("介质清单中的文件路径无效，请重新写入 USB").to_string())?;
    let path = root.join(relative);
    let meta = fs::symlink_metadata(&path)
        .map_err(|_| String::from("介质中缺少该文件，请重新写入 USB"))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(String::from("介质中的条目不是普通文件，请重新写入 USB"));
    }
    Ok(path)
}

fn entry_file_name(entry: &MediaEntry) -> Result<String, String> {
    let relative = safe_relative(&entry.file)
        .ok_or_else(|| String::from("介质清单中的文件路径无效，请重新写入 USB").to_string())?;
    relative
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| String::from("介质清单中的文件名无效，请重新写入 USB").to_string())
}

fn safe_relative(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') || trimmed.contains('\\') {
        return None;
    }
    let mut path = PathBuf::new();
    for part in trimmed.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
        if part.contains(':') || part.contains('\0') {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

fn load_manifest(root: &Path) -> Result<Option<MediaManifest>, String> {
    let path = root.join(MANIFEST_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .map_err(|_| String::from("无法读取介质清单，请确认 USB 已连接且未被占用"))?;
    let manifest = serde_json::from_str::<MediaManifest>(&content)
        .map_err(|_| String::from("介质清单已损坏，请在资源管理器中删除 media.json 后重新写入"))?;
    Ok(Some(manifest))
}

fn save_manifest(root: &Path, manifest: &MediaManifest) -> Result<(), String> {
    let content = serde_json::to_string_pretty(manifest)
        .map_err(|error| format!("生成介质清单失败：{error}"))?;
    atomic_file::write_text_atomic(
        &root.join(MANIFEST_FILE),
        &content,
        atomic_file::FilePermission::Default,
    )
    .map_err(|error| format!("保存介质清单失败：{error}"))
}

fn read_bundle_metadata(dir: &Path) -> Option<BundleMetadata> {
    let content = fs::read_to_string(dir.join("bundle.json")).ok()?;
    let value: Value = serde_json::from_str(&content).ok()?;
    let metadata = value.get("metadata")?;
    let product = string_field(metadata, "product");
    let version = string_field(metadata, "version");
    if product.trim().is_empty() || version.trim().is_empty() {
        return None;
    }
    Some(BundleMetadata {
        product,
        display_name: string_field(metadata, "displayName"),
        version,
        commit: string_field(metadata, "commit"),
        commit_short: string_field(metadata, "commitShort"),
    })
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn display_label(meta: &BundleMetadata) -> String {
    let display = meta.display_name.trim();
    if !display.is_empty() {
        return display.to_string();
    }
    let product = meta.product.trim();
    if !product.is_empty() {
        return product.to_string();
    }
    String::from("交付包")
}

fn label_for_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

fn is_image_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".tar") || lower.ends_with(".tgz") || lower.ends_with(".tar.gz")
}

fn sanitize_name(raw: &str, fallback: &str) -> String {
    let mut sanitized: String = raw
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                character
            }
        })
        .collect();
    sanitized = sanitized.trim().trim_matches('.').trim().to_string();
    if sanitized.is_empty() {
        return fallback.to_string();
    }
    if sanitized.chars().count() > 72 {
        sanitized = sanitized.chars().take(72).collect();
    }
    sanitized
}

fn unique_name(used: &mut Vec<String>, candidate: &str) -> String {
    let candidate = sanitize_name(candidate, "文件");
    let mut name = candidate.clone();
    let mut index = 2;
    while used.iter().any(|item| item == &name) {
        let (stem, extension) = split_extension(&candidate);
        name = format!("{stem}-{index}{extension}");
        index += 1;
    }
    used.push(name.clone());
    name
}

fn split_extension(name: &str) -> (String, String) {
    let lower = name.to_ascii_lowercase();
    for extension in [".tar.gz", ".tgz", ".tar"] {
        if lower.ends_with(extension) {
            let stem = name[..name.len() - extension.len()].to_string();
            return (stem, name[name.len() - extension.len()..].to_string());
        }
    }
    (name.to_string(), String::new())
}

fn temporary_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    target.with_file_name(format!(".{name}.part-{}", std::process::id()))
}

fn entry_id(kind: &str, digest: &str) -> String {
    let prefix: String = digest.chars().take(16).collect();
    format!("{kind}-{prefix}")
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

fn emit(
    channel: &Channel<UsbProgress>,
    phase: &str,
    current: usize,
    total: usize,
    label: &str,
    percent: usize,
    message: &str,
    finished: bool,
) {
    let _ = channel.send(UsbProgress {
        phase: phase.to_string(),
        current,
        total,
        label: label.to_string(),
        percent: percent.min(100),
        message: message.to_string(),
        finished,
    });
}

fn progress_percent(index: usize, total: usize) -> usize {
    if total == 0 {
        100
    } else {
        index.saturating_mul(100) / total
    }
}

fn percent_of(done: u64, total: u64) -> usize {
    if total == 0 {
        100
    } else {
        ((done.saturating_mul(100)) / total).min(100) as usize
    }
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn timestamp_text() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            total += dir_size(&path);
        } else if meta.is_file() {
            total += meta.len();
        }
    }
    total
}

fn modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
    bytes: u64,
}

impl<W> HashingWriter<W> {
    fn digest(&self) -> String {
        hex_digest(self.hasher.clone().finalize())
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.hasher.update(&buffer[..written]);
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_reject_escape_and_absolute_forms() {
        assert!(safe_relative("artifacts/foo.tgz").is_some());
        assert!(safe_relative("../media.json").is_none());
        assert!(safe_relative("artifacts/../../media.json").is_none());
        assert!(safe_relative("/tmp/foo.tgz").is_none());
        assert!(safe_relative("artifacts\\foo.tgz").is_none());
        assert!(safe_relative("C:/foo.tgz").is_none());
        assert!(safe_relative("").is_none());
    }

    #[test]
    fn media_root_is_created_inside_the_selected_directory() {
        let selected = Path::new("/tmp/usb");
        assert_eq!(media_root(selected), Path::new("/tmp/usb").join(MEDIA_ROOT_NAME));
        let inside = Path::new("/tmp/usb").join(MEDIA_ROOT_NAME);
        assert_eq!(media_root(&inside), inside);
    }

    #[test]
    fn file_names_keep_readable_text_and_drop_invalid_characters() {
        assert_eq!(sanitize_name("低配版 1.0.0", "交付包"), "低配版 1.0.0");
        assert_eq!(sanitize_name("a/b:c*?", "交付包"), "a_b_c__");
        assert_eq!(sanitize_name("...", "交付包"), "交付包");
        assert_eq!(sanitize_name("", "交付包"), "交付包");
    }

    #[test]
    fn duplicate_names_get_numeric_suffixes() {
        let mut used = Vec::new();
        assert_eq!(unique_name(&mut used, "app-1.0.0.tgz"), "app-1.0.0.tgz");
        assert_eq!(unique_name(&mut used, "app-1.0.0.tgz"), "app-1.0.0-2.tgz");
        assert_eq!(unique_name(&mut used, "app-1.0.0.tgz"), "app-1.0.0-3.tgz");
    }

    #[test]
    fn exported_artifact_round_trips_through_the_media_manifest() {
        let base = std::env::temp_dir().join(format!(
            "usb-media-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bundle_dir = base.join("bundle");
        fs::create_dir_all(bundle_dir.join("app")).unwrap();
        fs::write(
            bundle_dir.join("bundle.json"),
            r#"{"metadata":{"product":"demo","displayName":"演示产品","version":"1.2.3","commit":"abcdef123456"}}"#,
        )
        .unwrap();
        fs::write(bundle_dir.join("app/run.sh"), b"#!/bin/sh\necho ok\n").unwrap();

        let root = base.join("usb").join(MEDIA_ROOT_NAME);
        fs::create_dir_all(root.join(ARTIFACT_DIR)).unwrap();
        fs::create_dir_all(root.join(IMAGE_DIR)).unwrap();
        let roots = MediaRoots {
            selected: base.join("usb"),
            artifacts: root.join(ARTIFACT_DIR),
            images: root.join(IMAGE_DIR),
            root: root.clone(),
        };

        let mut used = Vec::new();
        let entry = write_artifact(&roots, &bundle_dir, &mut used, "2026-01-01T00:00:00Z", |_| {})
            .expect("写入交付包");
        assert_eq!(entry.kind, "artifact");
        assert_eq!(entry.version, "1.2.3");
        assert_eq!(entry.label, "演示产品");

        let archive = root.join(&entry.file);
        assert!(archive.is_file());
        assert_eq!(entry.sha256, super::super::bundle::sha256_path(&archive).unwrap());
        assert_eq!(fs::metadata(&archive).unwrap().len(), entry.size_bytes);

        let reader = std::fs::File::open(&archive).unwrap();
        let mut tar = tar::Archive::new(flate2::read::MultiGzDecoder::new(reader));
        let names: Vec<String> = tar
            .entries()
            .unwrap()
            .flatten()
            .map(|item| item.path().unwrap().display().to_string())
            .collect();
        assert!(names.iter().any(|name| name == "bundle.json"));
        assert!(names.iter().any(|name| name == "app/run.sh"));

        let view = entry_view(&root, &entry);
        assert!(view.present);
        assert!(view.issue.is_empty());

        let manifest = MediaManifest {
            schema_version: SCHEMA_VERSION,
            updated_at: String::from("2026-01-01T00:00:00Z"),
            entries: vec![entry.clone()],
        };
        save_manifest(&root, &manifest).expect("保存介质清单");
        let scan = scan_usb_media_blocking(&roots.selected.display().to_string())
            .expect("读取介质");
        assert!(scan.exists);
        assert!(scan.writable);
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.entries[0].present);
        assert_eq!(scan.entries[0].size_bytes, entry.size_bytes);

        fs::remove_file(&archive).unwrap();
        let missing = scan_usb_media_blocking(&roots.selected.display().to_string())
            .expect("读取介质");
        assert!(!missing.entries[0].present);
        assert!(missing.entries[0].issue.contains("缺少"));

        let _ = fs::remove_dir_all(&base);
    }
}
