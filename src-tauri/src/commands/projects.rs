//! 项目模型：一个项目对应一套部署目标（主机 + 部署目录 + 制品源）。
//!
//! 项目拓扑（服务、数据库、产物映射）来自交付包描述符，本模块只保存定位信息。
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use std::time::SystemTime;

use super::bundle::{Bundle, BundleDescriptor};
use super::remote_ssh::{
    base64_encode, ensure_known_host, run_ssh_script_strict_with_timeout, shell_quote,
};
use crate::core::path_guard;

const PROJECTS_FILE: &str = "projects.json";
const PROJECT_ENV_DIR: &str = "project-envs";
const MAX_ENV_BYTES: usize = 256 * 1024;

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
    let dir = crate::core::config::app_config_dir(app).ok();
    if let Some(dir) = &dir {
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
    match crate::core::json_store::load::<ProjectStore>(&path, "项目配置") {
        Ok(store) => store,
        Err(error) => {
            log::warn!("{error}");
            None
        }
    }
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
    let loaded = match crate::core::json_store::load::<ProjectStore>(&path, "项目配置") {
        Ok(Some(loaded)) => loaded,
        Ok(None) => {
            remember_stamp(file_stamp(&path));
            return;
        }
        Err(error) => {
            log::warn!("{error}");
            remember_stamp(file_stamp(&path));
            return;
        }
    };
    if let Ok(mut current) = store().lock() {
        *current = loaded;
    }
    remember_stamp(Some(stamp));
    invalidate_bundle_cache();
}

/// 在跨进程锁内按磁盘最新内容修改项目配置，并同步内存快照。
///
/// MCP 服务会改写同一份 `projects.json`，因此不能拿内存里的旧快照直接覆盖磁盘：
/// 锁内重新读取，修改完成后再写回，写入成功后才刷新内存与交付包缓存。
fn mutate_store<F>(mutate: F) -> Result<ProjectStore, String>
where
    F: FnOnce(&mut ProjectStore) -> Result<(), String>,
{
    let path = store_path()?;
    let updated = crate::core::json_store::update(
        &path,
        ProjectStore::default(),
        "项目配置",
        crate::core::atomic_file::FilePermission::Default,
        mutate,
    )?;
    remember_stamp(file_stamp(&path));
    if let Ok(mut guard) = store().lock() {
        *guard = updated.clone();
    }
    invalidate_bundle_cache();
    Ok(updated)
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
    let target = root.to_string_lossy().trim_end_matches('/').to_string();
    if snapshot()
        .projects
        .iter()
        .all(|project| !points_into(project, &target))
    {
        return;
    }
    let result = mutate_store(|current| {
        for project in current.projects.iter_mut() {
            if points_into(project, &target) {
                project.last_bundle_dir.clear();
                project.last_version.clear();
            }
        }
        Ok(())
    });
    if let Err(error) = result {
        log::warn!("清理已删除交付包的项目绑定失败：{error}");
    }
}

fn points_into(project: &Project, target: &str) -> bool {
    let dir = project.last_bundle_dir.trim().trim_end_matches('/');
    !dir.is_empty() && (dir == target || dir.starts_with(&format!("{target}/")))
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
    mutate_store(|current| {
        let target = current
            .projects
            .iter()
            .find(|project| project.id == id)
            .ok_or_else(|| "所选项目不存在，请刷新后重试".to_string())?;
        current.active_project_id = target.id.clone();
        log::info!(target: "project", "切换当前项目：{} ({})", target.name, target.id);
        Ok(())
    })
}

#[tauri::command]
pub fn get_project_env(project_id: String) -> Result<String, String> {
    if !project_exists(&project_id) {
        return Err("所选项目不存在，请刷新后重试".to_string());
    }
    let path = project_env_path(&project_id)?;
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("读取项目环境变量失败：{error}")),
    }
}

#[tauri::command]
pub fn save_project_env(project_id: String, content: String) -> Result<(), String> {
    if !project_exists(&project_id) {
        return Err("所选项目不存在，请刷新后重试".to_string());
    }
    let content = normalize_env_content(&content)?;
    let path = project_env_path(&project_id)?;
    if content.trim().is_empty() {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("删除项目环境变量失败：{error}")),
        };
    }
    write_private_env(&path, &content)
}

#[tauri::command]
pub fn create_project(input: ProjectInput) -> Result<ProjectStore, String> {
    let name = validate_name(&input.name)?;
    let install_dir = validate_project_install_dir(&input.host_id, &input.install_dir)?;
    validate_host(&input.host_id)?;
    mutate_store(|current| {
        let project = Project {
            id: next_id(current),
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
        if current.active_project_id.trim().is_empty() {
            current.active_project_id = project.id.clone();
        }
        current.projects.push(project);
        current.schema_version = 1;
        Ok(())
    })
}

#[tauri::command]
pub fn update_project(input: ProjectInput) -> Result<ProjectStore, String> {
    let id = input.id.trim().to_string();
    if id.is_empty() {
        return Err("缺少项目标识，请刷新后重试".to_string());
    }
    let name = validate_name(&input.name)?;
    let install_dir = validate_project_install_dir(&input.host_id, &input.install_dir)?;
    validate_host(&input.host_id)?;
    mutate_store(|current| {
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
        Ok(())
    })
}

#[tauri::command]
pub fn delete_project(id: String) -> Result<ProjectStore, String> {
    let store = mutate_store(|current| {
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
        Ok(())
    })?;
    if let Ok(path) = project_env_path(&id) {
        let _ = fs::remove_file(path);
    }
    Ok(store)
}

#[tauri::command]
pub async fn open_local_directory(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || open_local_directory_blocking(path))
        .await
        .map_err(|_| "打开部署目录任务未完成".to_string())?
}

fn open_local_directory_blocking(raw: String) -> Result<(), String> {
    let path = validate_open_directory(&raw)?;
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(&path);
    command
        .spawn()
        .map_err(|error| format!("打开部署目录失败：{error}"))?;
    Ok(())
}

fn validate_open_directory(raw: &str) -> Result<PathBuf, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("请填写部署目录".to_string());
    }
    if value.contains(['\n', '\r', '\0']) {
        return Err("部署目录包含不支持的字符".to_string());
    }
    if !path_guard::is_absolute(value) {
        return Err("部署目录必须填写绝对路径".to_string());
    }
    if path_guard::has_parent_dir(Path::new(value)) {
        return Err("部署目录不能包含上级目录".to_string());
    }
    let path = PathBuf::from(value);
    if !path.is_dir() {
        return Err("部署目录不存在或不可访问，请检查项目配置后重试".to_string());
    }
    Ok(path)
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
    mutate_store(|current| {
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
        Ok(())
    })
}

fn project_exists(project_id: &str) -> bool {
    let id = project_id.trim();
    !id.is_empty() && snapshot().projects.iter().any(|project| project.id == id)
}

fn project_env_path(project_id: &str) -> Result<PathBuf, String> {
    let id = project_id.trim();
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("项目标识无效，请刷新后重试".to_string());
    }
    let dir = CONFIG_DIR
        .get()
        .ok_or_else(|| "应用配置目录不可用".to_string())?;
    Ok(dir.join(PROJECT_ENV_DIR).join(format!("{id}.env")))
}

fn normalize_env_content(raw: &str) -> Result<String, String> {
    if raw.as_bytes().len() > MAX_ENV_BYTES {
        return Err("环境变量文件不能超过 256 KB".to_string());
    }
    if raw.contains('\0') {
        return Err("环境变量文件不能包含空字符".to_string());
    }
    let mut content = raw.replace("\r\n", "\n");
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    Ok(content)
}

pub(crate) fn write_private_env(path: &Path, content: &str) -> Result<(), String> {
    write_private_env_bytes(path, content.as_bytes())
}

pub(crate) fn write_private_env_bytes(path: &Path, content: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建项目环境变量目录失败：{error}"))?;
    }
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let temporary = path.with_extension(format!("tmp-{}-{stamp}", std::process::id()));
    // 以排他方式创建临时文件，并在创建时就带上 0600（Unix），避免秘密文件
    // 在写入权限之前短暂地以默认权限存在。
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()
    };
    if let Err(error) = write() {
        let _ = fs::remove_file(&temporary);
        return Err(format!("写入项目环境变量失败：{error}"));
    }
    if let Err(error) = fs::rename(&temporary, path) {
        // Windows 不允许 rename 覆盖已存在的文件，先移除目标再重试。
        let replaced = if path.is_file() {
            fs::remove_file(path).and_then(|_| fs::rename(&temporary, path))
        } else {
            Err(error)
        };
        if let Err(error) = replaced {
            let _ = fs::remove_file(&temporary);
            return Err(format!("保存项目环境变量失败：{error}"));
        }
    }
    Ok(())
}

pub(crate) fn read_active_project_env() -> Result<Option<String>, String> {
    let Some(project) = active_project() else {
        return Ok(None);
    };
    let path = project_env_path(&project.id)?;
    match fs::read_to_string(path) {
        Ok(content) => Ok((!content.trim().is_empty()).then_some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取项目环境变量失败：{error}")),
    }
}

pub(crate) fn write_active_project_env_to_dir(deploy_dir: &Path) -> Result<bool, String> {
    let Some(content) = read_active_project_env()? else {
        return Ok(false);
    };
    let canonical = fs::canonicalize(deploy_dir)
        .map_err(|_| String::from("部署目录不存在或不可访问，无法写入环境变量文件"))?;
    if canonical.parent().is_none() {
        return Err(String::from(
            "部署目录不能是系统根目录，请填写专用部署路径",
        ));
    }
    let path = canonical.join(".env");
    write_private_env(&path, &content)?;
    log::info!(target: "deploy", "已写入项目环境变量：{}", path.display());
    Ok(true)
}

pub(crate) fn upload_active_project_env(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    deploy_dir: &str,
) -> Result<bool, String> {
    let Some(content) = read_active_project_env()? else {
        return Ok(false);
    };
    validate_remote_env_deploy_dir(deploy_dir)?;
    let dir = deploy_dir.trim().trim_end_matches('/');
    // 环境变量可能包含密码与令牌，上传前必须确认目标主机指纹已经在本机验证。
    ensure_known_host(host, port)?;
    let script = env_upload_script(dir, &content)?;
    let output = run_ssh_script_strict_with_timeout(
        host,
        port,
        user,
        key,
        &script,
        Duration::from_secs(120),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        if stderr.contains("host key verification failed") {
            return Err(String::from(
                "目标主机指纹校验失败，可能已被替换。请先在本机终端连接该主机并确认指纹后再重试",
            ));
        }
        return Err(String::from(
            "上传项目环境变量失败，请检查目标主机指纹与部署目录权限后重试",
        ));
    }
    log::info!(target: "deploy", "已上传项目环境变量：{dir}/.env");
    Ok(true)
}

fn validate_remote_env_deploy_dir(raw: &str) -> Result<(), String> {
    let value = raw.trim().trim_end_matches('/');
    if value.is_empty() || value.contains(['\n', '\r', '\0']) || !value.starts_with('/') {
        return Err(String::from("目标部署目录无效，无法写入环境变量文件"));
    }
    let mut has_normal = false;
    for part in Path::new(value).components() {
        match part {
            Component::Normal(_) => has_normal = true,
            Component::RootDir => {}
            _ => return Err(String::from("目标部署目录无效，无法写入环境变量文件")),
        }
    }
    if !has_normal {
        return Err(String::from(
            "目标部署目录不能是系统根目录，请填写专用部署路径",
        ));
    }
    Ok(())
}

fn env_upload_script(dir: &str, content: &str) -> Result<String, String> {
    let target = format!("{dir}/.env");
    let template = format!("{dir}/.env.XXXXXX");
    let encoded = base64_encode(content)?;
    Ok(format!(
        "set -e\numask 077\nmkdir -p {dir}\ntmp=$(mktemp {template})\ntrap 'rm -f \"$tmp\"' EXIT\nprintf '%s' {encoded} | base64 -d > \"$tmp\"\nmv \"$tmp\" {target}\ntrap - EXIT\n",
        dir = shell_quote(dir),
        encoded = shell_quote(&encoded),
        template = shell_quote(&template),
        target = shell_quote(&target)
    ))
}

/// 记录部署目录 `.env` 的原始内容，在预检失败或只做预检时恢复，避免误改部署文件。
pub(crate) struct ProjectEnvGuard {
    deploy_dir: PathBuf,
    previous: Option<Vec<u8>>,
    armed: bool,
}

impl ProjectEnvGuard {
    pub(crate) fn capture(deploy_dir: &Path) -> Result<Self, String> {
        let deploy_dir = fs::canonicalize(deploy_dir)
            .map_err(|_| String::from("部署目录不存在或不可访问，无法读取环境变量"))?;
        let path = deploy_dir.join(".env");
        let previous = match fs::read(&path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("读取部署目录环境变量失败：{error}")),
        };
        Ok(Self {
            deploy_dir,
            previous,
            armed: true,
        })
    }

    pub(crate) fn previous_content(&self) -> Option<&[u8]> {
        self.previous.as_deref()
    }

    pub(crate) fn restore(&self) -> Result<(), String> {
        let path = self.deploy_dir.join(".env");
        match self.previous.as_deref() {
            Some(content) => write_private_env_bytes(&path, content),
            None => match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("恢复部署目录环境变量失败：{error}")),
            },
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProjectEnvGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self.restore() {
                log::warn!(target: "deploy", "恢复部署目录环境变量失败：{error}");
            }
        }
    }
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
    if !path_guard::is_absolute(value) {
        return Err("部署目录必须填写绝对路径".to_string());
    }
    if path_guard::has_parent_dir(Path::new(value)) {
        return Err("部署目录不能包含上级目录".to_string());
    }
    Ok(value.to_string())
}

fn validate_project_install_dir(host_id: &str, raw: &str) -> Result<String, String> {
    let value = validate_install_dir(raw)?;
    if host_is_local(host_id)? {
        validate_local_install_dir(&value)?;
    } else {
        validate_remote_env_deploy_dir(&value)?;
    }
    Ok(value)
}

pub(crate) fn validate_local_install_dir(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    let has_normal = path
        .components()
        .any(|part| matches!(part, Component::Normal(_)));
    if !path.is_absolute() || !has_normal || path_guard::has_parent_dir(path) {
        return Err(String::from(
            "部署目录必须是当前系统可用的绝对路径，且不能是系统根目录",
        ));
    }
    Ok(())
}

fn host_is_local(host_id: &str) -> Result<bool, String> {
    let id = host_id.trim();
    let dir = CONFIG_DIR
        .get()
        .ok_or_else(|| String::from("应用配置目录不可用"))?;
    let text = fs::read_to_string(dir.join("connections.json")).unwrap_or_default();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let local = value
        .get("hosts")
        .and_then(|hosts| hosts.as_array())
        .and_then(|hosts| {
            hosts
                .iter()
                .find(|host| host.get("id").and_then(serde_json::Value::as_str) == Some(id))
        })
        .map(|host| {
            host.get("kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "local")
        })
        .unwrap_or(false);
    Ok(local)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_directory_rejects_unsafe_paths() {
        assert!(validate_open_directory("").is_err());
        assert!(validate_open_directory("relative/path").is_err());
        assert!(validate_open_directory("/tmp/../etc").is_err());
        assert!(validate_open_directory("/path/that/does/not/exist").is_err());
    }

    #[test]
    fn open_directory_accepts_existing_directory() {
        let dir = std::env::temp_dir();
        assert!(validate_open_directory(dir.to_str().unwrap()).is_ok());
    }

    #[test]
    fn project_env_is_normalized_and_written_privately() {
        assert_eq!(
            normalize_env_content("A=1\r\nB=2").unwrap(),
            "A=1\nB=2\n"
        );
        assert!(normalize_env_content("A=\0").is_err());
        assert!(normalize_env_content(&"x".repeat(MAX_ENV_BYTES + 1)).is_err());

        let dir = std::env::temp_dir().join(format!(
            "deploy-project-env-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        let path = dir.join(".env");
        write_private_env(&path, "A=1\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "A=1\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn project_env_guard_restores_previous_state() {
        let dir = std::env::temp_dir().join(format!(
            "deploy-project-env-guard-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(&path, "OLD=1\n").unwrap();
        {
            let _guard = ProjectEnvGuard::capture(&dir).unwrap();
            write_private_env(&path, "NEW=2\n").unwrap();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "OLD=1\n");

        fs::remove_file(&path).unwrap();
        {
            let _guard = ProjectEnvGuard::capture(&dir).unwrap();
            write_private_env(&path, "NEW=2\n").unwrap();
        }
        assert!(!path.exists());

        {
            let mut guard = ProjectEnvGuard::capture(&dir).unwrap();
            write_private_env(&path, "KEEP=1\n").unwrap();
            guard.disarm();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "KEEP=1\n");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deploy_dir_validation_rejects_root_and_cross_platform_paths() {
        assert!(validate_local_install_dir("/").is_err());
        assert!(validate_local_install_dir("/.").is_err());
        assert!(validate_local_install_dir("///").is_err());
        assert!(validate_local_install_dir("relative/path").is_err());
        #[cfg(unix)]
        {
            assert!(validate_local_install_dir("/opt/demo").is_ok());
            assert!(validate_local_install_dir("C:\\deploy").is_err());
            assert!(validate_local_install_dir("\\\\server\\share").is_err());
        }
        #[cfg(windows)]
        {
            assert!(validate_local_install_dir("C:\\deploy\\demo").is_ok());
            assert!(validate_local_install_dir("\\\\server\\share").is_err());
        }

        assert!(validate_remote_env_deploy_dir("/").is_err());
        assert!(validate_remote_env_deploy_dir("/.").is_err());
        assert!(validate_remote_env_deploy_dir("///").is_err());
        assert!(validate_remote_env_deploy_dir("C:\\deploy").is_err());
        assert!(validate_remote_env_deploy_dir("\\\\server\\share").is_err());
        assert!(validate_remote_env_deploy_dir("/opt/demo").is_ok());
    }

    #[test]
    fn project_env_path_rejects_traversal_and_remote_upload_hides_content() {
        assert!(project_env_path("../outside").is_err());
        assert!(project_env_path("p/../../x").is_err());

        let script = env_upload_script("/srv/demo/deploy", "DB_PASSWORD=secret\n").unwrap();
        assert!(script.contains("/srv/demo/deploy/.env"));
        assert!(script.contains("base64 -d"));
        assert!(script.contains("umask 077"));
        assert!(!script.contains("DB_PASSWORD=secret"));
    }
}
