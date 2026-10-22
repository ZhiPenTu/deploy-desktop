//! 项目模型：一个项目对应一套部署目标（主机 + 部署目录 + 制品源）。
//!
//! 项目拓扑（服务、数据库、产物映射）来自交付包描述符，本模块只保存定位信息。
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use super::bundle::{Bundle, BundleDescriptor};

const PROJECTS_FILE: &str = "projects.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub product: String,
    pub host_id: String,
    pub install_dir: String,
    pub source_id: String,
    pub last_version: String,
    pub last_bundle_dir: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectStore {
    pub schema_version: u32,
    pub active_project_id: String,
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectInput {
    pub id: String,
    pub name: String,
    pub host_id: String,
    pub install_dir: String,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleSummary {
    pub product: String,
    pub display_name: String,
    pub version: String,
    pub branch: String,
    pub commit_short: String,
    pub built_at: String,
    pub container_prefix: String,
    pub compose: String,
    pub variant: String,
    pub components: Vec<super::bundle::ComponentSpec>,
    pub services: Vec<super::bundle::ServiceSpec>,
    pub database: Option<super::bundle::DatabaseSpec>,
    pub images: Vec<super::bundle::ImageSpec>,
    pub artifacts: Vec<super::bundle::ArtifactSpec>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveProjectContext {
    pub project: Option<Project>,
    pub bundle: Option<BundleSummary>,
    /// 目标机上 Compose 清单所在目录，由部署目录与包内清单位置推出。
    pub deploy_dir: String,
}

static STORE: OnceLock<Mutex<ProjectStore>> = OnceLock::new();
static STORE_STAMP: OnceLock<Mutex<Option<(SystemTime, u64)>>> = OnceLock::new();
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

struct BundleCache {
    project_id: String,
    dir: String,
    stamp: u128,
    bundle: Arc<Bundle>,
}

static BUNDLE_CACHE: Mutex<Option<BundleCache>> = Mutex::new(None);

pub fn init_projects(app: &tauri::AppHandle) {
    use tauri::Manager;
    let dir = app.path().app_config_dir().ok();
    if let Some(dir) = &dir {
        let _ = fs::create_dir_all(dir);
        let _ = CONFIG_DIR.set(dir.clone());
    }
    let store = dir
        .as_deref()
        .map(load_store)
        .unwrap_or_default()
        .unwrap_or_default();
    let _ = STORE.set(Mutex::new(store));
    let _ = STORE_STAMP.set(Mutex::new(dir.as_deref().and_then(store_stamp)));
    invalidate_bundle_cache();
}

fn store() -> &'static Mutex<ProjectStore> {
    STORE.get_or_init(|| Mutex::new(ProjectStore::default()))
}

fn store_path() -> Result<PathBuf, String> {
    let dir = CONFIG_DIR
        .get()
        .cloned()
        .ok_or_else(|| "应用配置目录不可用".to_string())?;
    Ok(dir.join(PROJECTS_FILE))
}

fn load_store(dir: &Path) -> Option<ProjectStore> {
    let path = dir.join(PROJECTS_FILE);
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn file_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn store_stamp(dir: &Path) -> Option<(SystemTime, u64)> {
    file_stamp(&dir.join(PROJECTS_FILE))
}

fn stamp_slot() -> &'static Mutex<Option<(SystemTime, u64)>> {
    STORE_STAMP.get_or_init(|| Mutex::new(None))
}

fn remember_stamp(stamp: Option<(SystemTime, u64)>) {
    if let Ok(mut guard) = stamp_slot().lock() {
        *guard = stamp;
    }
}

/// 项目配置可能被 MCP 服务或用户手动修改，每次读取前比对文件时间与大小，
/// 发现变化就重新载入，避免界面用旧快照覆盖外部修改。
fn reload_if_changed() {
    let Ok(path) = store_path() else { return };
    let Some(stamp) = file_stamp(&path) else {
        return;
    };
    let changed = stamp_slot()
        .lock()
        .map(|guard| *guard != Some(stamp))
        .unwrap_or(false);
    if !changed {
        return;
    }
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => {
            remember_stamp(Some(stamp));
            return;
        }
    };
    let Ok(loaded) = serde_json::from_str::<ProjectStore>(text.as_str()) else {
        remember_stamp(Some(stamp));
        return;
    };
    if let Ok(mut current) = store().lock() {
        *current = loaded;
    }
    remember_stamp(Some(stamp));
    invalidate_bundle_cache();
}

fn persist(current: &ProjectStore) -> Result<(), String> {
    let path = store_path()?;
    let text = serde_json::to_string_pretty(current)
        .map_err(|_| "无法生成项目配置".to_string())?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text).map_err(|_| "无法写入项目配置".to_string())?;
    fs::rename(&temp, &path).map_err(|_| "无法保存项目配置".to_string())?;
    remember_stamp(file_stamp(&path));
    Ok(())
}

pub fn snapshot() -> ProjectStore {
    reload_if_changed();
    store()
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

pub fn active_project() -> Option<Project> {
    let current = snapshot();
    let id = current.active_project_id.trim().to_string();
    if id.is_empty() {
        return current.projects.first().cloned();
    }
    current
        .projects
        .into_iter()
        .find(|project| project.id == id)
}

pub fn active_bundle() -> Option<Arc<Bundle>> {
    let project = active_project()?;
    let dir = project.last_bundle_dir.trim().to_string();
    if dir.is_empty() {
        return None;
    }
    let stamp = fs::metadata(Path::new(&dir).join("bundle.json"))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|value| value.as_millis())
        .unwrap_or(0);
    if let Ok(guard) = BUNDLE_CACHE.lock() {
        if let Some(cache) = guard.as_ref() {
            if cache.project_id == project.id && cache.dir == dir && cache.stamp == stamp {
                return Some(cache.bundle.clone());
            }
        }
    }
    let bundle = Arc::new(Bundle::load(Path::new(&dir)).ok()?);
    if let Ok(mut guard) = BUNDLE_CACHE.lock() {
        *guard = Some(BundleCache {
            project_id: project.id,
            dir,
            stamp,
            bundle: bundle.clone(),
        });
    }
    Some(bundle)
}

pub fn invalidate_bundle_cache() {
    if let Ok(mut guard) = BUNDLE_CACHE.lock() {
        *guard = None;
    }
}

/// 本地交付包被删除后，清空仍指向该目录的项目绑定。
pub fn clear_bundle_dir(root: &Path) {
    let mut current = snapshot();
    let target = root.to_string_lossy().trim_end_matches('/').to_string();
    let mut changed = false;
    for project in current.projects.iter_mut() {
        let dir = project.last_bundle_dir.trim().trim_end_matches('/').to_string();
        if !dir.is_empty() && (dir == target || dir.starts_with(&format!("{target}/"))) {
            project.last_bundle_dir.clear();
            project.last_version.clear();
            changed = true;
        }
    }
    if !changed {
        return;
    }
    if persist(&current).is_ok() {
        if let Ok(mut guard) = store().lock() {
            *guard = current;
        }
        invalidate_bundle_cache();
    }
}

fn summarize(descriptor: &BundleDescriptor) -> BundleSummary {
    BundleSummary {
        product: descriptor.metadata.product.clone(),
        display_name: descriptor.metadata.display_name.clone(),
        version: descriptor.metadata.version.clone(),
        branch: descriptor.metadata.branch.clone(),
        commit_short: descriptor.metadata.commit_short.clone(),
        built_at: descriptor.metadata.built_at.clone(),
        container_prefix: descriptor.runtime.container_prefix.trim().to_string(),
        compose: descriptor.runtime.compose.trim().to_string(),
        variant: descriptor.runtime.variant.trim().to_string(),
        components: descriptor.components.clone(),
        services: descriptor.runtime.services.clone(),
        database: descriptor.runtime.database.clone(),
        images: descriptor.images.clone(),
        artifacts: descriptor.artifacts.clone(),
    }
}

#[tauri::command]
pub fn list_projects() -> ProjectStore {
    snapshot()
}

#[tauri::command]
pub fn get_active_project_context() -> ActiveProjectContext {
    let project = active_project();
    let bundle = active_bundle();
    let deploy_dir = match (&project, &bundle) {
        (Some(project), Some(bundle)) => Path::new(&project.install_dir)
            .join(bundle.deploy_source_rel())
            .to_string_lossy()
            .to_string(),
        _ => String::new(),
    };
    ActiveProjectContext {
        project,
        bundle: bundle.map(|bundle| summarize(&bundle.descriptor)),
        deploy_dir,
    }
}

#[tauri::command]
pub fn set_active_project(id: String) -> Result<ProjectStore, String> {
    let mut current = snapshot();
    let target = current
        .projects
        .iter()
        .find(|project| project.id == id)
        .ok_or_else(|| "所选项目不存在，请刷新后重试".to_string())?
        .clone();
    current.active_project_id = target.id.clone();
    log::info!(target: "project", "切换当前项目：{} ({})", target.name, target.id);
    persist(&current)?;
    *store().lock().map_err(|_| "项目配置不可用".to_string())? = current.clone();
    invalidate_bundle_cache();
    Ok(current)
}

#[tauri::command]
pub fn create_project(input: ProjectInput) -> Result<ProjectStore, String> {
    let mut current = snapshot();
    let name = validate_name(&input.name)?;
    let install_dir = validate_install_dir(&input.install_dir)?;
    validate_host(&input.host_id)?;
    let project = Project {
        id: next_id(&current),
        name,
        product: String::new(),
        host_id: input.host_id.trim().to_string(),
        install_dir,
        source_id: input.source_id.trim().to_string(),
        last_version: String::new(),
        last_bundle_dir: String::new(),
    };
    log::info!(
        target: "project",
        "新建项目：{} 目录={} 主机={} 制品源={}",
        project.name,
        project.install_dir,
        project.host_id,
        project.source_id
    );
    current.projects.push(project.clone());
    if current.active_project_id.trim().is_empty() {
        current.active_project_id = project.id;
    }
    current.schema_version = 1;
    persist(&current)?;
    *store().lock().map_err(|_| "项目配置不可用".to_string())? = current.clone();
    invalidate_bundle_cache();
    Ok(current)
}

#[tauri::command]
pub fn update_project(input: ProjectInput) -> Result<ProjectStore, String> {
    let mut current = snapshot();
    let id = input.id.trim().to_string();
    if id.is_empty() {
        return Err("缺少项目标识，请刷新后重试".to_string());
    }
    let name = validate_name(&input.name)?;
    let install_dir = validate_install_dir(&input.install_dir)?;
    validate_host(&input.host_id)?;
    let project = current
        .projects
        .iter_mut()
        .find(|project| project.id == id)
        .ok_or_else(|| "所选项目不存在，请刷新后重试".to_string())?;
    project.name = name;
    project.host_id = input.host_id.trim().to_string();
    project.install_dir = install_dir;
    project.source_id = input.source_id.trim().to_string();
    log::info!(
        target: "project",
        "更新项目：{} 目录={} 主机={}",
        project.name,
        project.install_dir,
        project.host_id
    );
    persist(&current)?;
    *store().lock().map_err(|_| "项目配置不可用".to_string())? = current.clone();
    invalidate_bundle_cache();
    Ok(current)
}

#[tauri::command]
pub fn delete_project(id: String) -> Result<ProjectStore, String> {
    let mut current = snapshot();
    let before = current.projects.len();
    current.projects.retain(|project| project.id != id);
    if current.projects.len() == before {
        return Err("所选项目不存在，请刷新后重试".to_string());
    }
    if current.active_project_id == id {
        current.active_project_id = current
            .projects
            .first()
            .map(|project| project.id.clone())
            .unwrap_or_default();
    }
    log::info!(target: "project", "删除项目：{id}");
    persist(&current)?;
    *store().lock().map_err(|_| "项目配置不可用".to_string())? = current.clone();
    invalidate_bundle_cache();
    Ok(current)
}

#[tauri::command]
pub fn activate_project_bundle(
    project_id: String,
    bundle_dir: String,
    version: String,
) -> Result<ProjectStore, String> {
    let bundle = Bundle::load(Path::new(bundle_dir.trim()))?;
    if !version.trim().is_empty() && bundle.version() != version.trim() {
        return Err(format!(
            "交付包版本与所选版本不一致（包内为 {}），请重新下载制品",
            bundle.version()
        ));
    }
    let mut current = snapshot();
    let project = current
        .projects
        .iter_mut()
        .find(|project| project.id == project_id)
        .ok_or_else(|| "所选项目不存在，请刷新后重试".to_string())?;
    if !project.product.trim().is_empty() && project.product.trim() != bundle.product() {
        return Err(format!(
            "该交付包属于「{}」，当前项目已绑定「{}」，请选择匹配的制品",
            bundle.product(),
            project.product
        ));
    }
    project.product = bundle.product().to_string();
    project.last_version = bundle.version().to_string();
    project.last_bundle_dir = bundle_dir.trim().to_string();
    persist(&current)?;
    *store().lock().map_err(|_| "项目配置不可用".to_string())? = current.clone();
    invalidate_bundle_cache();
    Ok(current)
}

fn validate_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("请填写项目名称".to_string());
    }
    if name.chars().count() > 60 {
        return Err("项目名称不能超过 60 个字符".to_string());
    }
    if name.contains(['\n', '\r', '\0']) {
        return Err("项目名称包含不支持的字符".to_string());
    }
    Ok(name.to_string())
}

fn validate_install_dir(raw: &str) -> Result<String, String> {
    let value = raw.trim().trim_end_matches('/');
    if value.is_empty() {
        return Err("请填写部署目录".to_string());
    }
    if value.contains(['\n', '\r', '\0']) {
        return Err("部署目录包含不支持的字符".to_string());
    }
    let windows_absolute = {
        let bytes = value.as_bytes();
        bytes.len() > 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/')
    };
    if !value.starts_with('/') && !windows_absolute {
        return Err("部署目录必须填写绝对路径".to_string());
    }
    if Path::new(value).components().any(|part| {
        matches!(part, std::path::Component::ParentDir)
    }) {
        return Err("部署目录不能包含上级目录".to_string());
    }
    Ok(value.to_string())
}

fn validate_host(host_id: &str) -> Result<(), String> {
    let id = host_id.trim();
    if id.is_empty() {
        return Err("请选择部署主机".to_string());
    }
    let dir = CONFIG_DIR
        .get()
        .ok_or_else(|| "应用配置目录不可用".to_string())?;
    let text = fs::read_to_string(dir.join("connections.json")).unwrap_or_default();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let exists = value
        .get("hosts")
        .and_then(|hosts| hosts.as_array())
        .map(|hosts| {
            hosts.iter().any(|host| {
                host.get("id")
                    .and_then(|item| item.as_str())
                    .map(|value| value == id)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if exists {
        Ok(())
    } else {
        Err("所选主机不存在，请先在主机管理中创建".to_string())
    }
}

fn next_id(current: &ProjectStore) -> String {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    let mut candidate = format!("p-{stamp}");
    let mut index = 1;
    while current.projects.iter().any(|project| project.id == candidate) {
        candidate = format!("p-{stamp}-{index}");
        index += 1;
    }
    candidate
}
