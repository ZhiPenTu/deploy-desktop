use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

const DEFAULT_UPDATE_ENDPOINT: &str =
    "https://github.com/ZhiPenTu/deploy-desktop/releases/latest/download/latest.json";

#[derive(Default)]
pub struct PendingUpdate(pub Mutex<Option<Update>>);

#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum UpdateProgress {
    #[serde(rename_all = "camelCase")]
    Started { content_length: Option<u64> },
    #[serde(rename_all = "camelCase")]
    Progress { chunk_length: usize },
    Finished,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadata {
    pub version: String,
    pub current_version: String,
    pub date: Option<i64>,
    pub body: Option<String>,
    pub install_blocker: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdateSettings {
    pub endpoint: String,
    pub allow_insecure: bool,
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("读取应用配置目录失败：{e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建应用配置目录失败：{e}"))?;
    Ok(dir.join("update-settings.json"))
}

fn friendly_update_error(message: String) -> String {
    let lower = message.to_lowercase();
    if lower.contains("404") || lower.contains("valid release json") {
        return format!("未能从 GitHub Releases 获取更新信息，仓库可能尚未发布版本（{message}）");
    }
    if lower.contains("error sending request")
        || lower.contains("timed out")
        || lower.contains("dns")
        || lower.contains("connect")
    {
        return "无法连接更新服务器，请检查网络连接后重试。".to_string();
    }
    format!("检查更新失败：{message}")
}

fn friendly_install_error(message: String) -> String {
    let lower = message.to_lowercase();
    if lower.contains("no such file or directory") || lower.contains("os error 2") {
        return "无法替换当前应用：应用所在位置不存在或已被移动。请将应用移动到“应用程序”文件夹后重新打开，再检查更新。".to_string();
    }
    if lower.contains("permission denied")
        || lower.contains("operation not permitted")
        || lower.contains("os error 1")
        || lower.contains("os error 13")
    {
        return "没有权限替换当前应用。请将应用移动到“应用程序”文件夹，或允许当前用户修改该应用后重试。".to_string();
    }
    if lower.contains("cross-device link") || lower.contains("os error 18") {
        return "更新文件与当前应用不在同一磁盘，无法直接替换。请将应用移动到“应用程序”文件夹后重试。".to_string();
    }
    if lower.contains("signature") {
        return "更新包签名校验失败，已停止安装。请从 GitHub Releases 重新下载完整安装包。"
            .to_string();
    }
    if lower.contains("error sending request")
        || lower.contains("timed out")
        || lower.contains("dns")
        || lower.contains("connect")
    {
        return "下载更新失败，请检查网络连接后重试。".to_string();
    }
    format!("安装更新失败：{message}")
}

#[cfg(target_os = "macos")]
fn install_blocker_for_bundle(bundle: &std::path::Path) -> Option<String> {
    if !bundle.exists() {
        return Some(
            "当前应用所在位置已不存在，可能已从磁盘映像或临时目录启动。请将应用移动到“应用程序”文件夹后重新打开，再检查更新。"
                .to_string(),
        );
    }
    let display = bundle.to_string_lossy();
    if display.starts_with("/Volumes/")
        || display.starts_with("/private/var/folders/")
        || display.starts_with("/var/folders/")
        || display.starts_with("/private/tmp/")
        || display.starts_with("/tmp/")
    {
        return Some(
            "当前应用运行在磁盘映像或临时目录中，无法自动更新。请先将应用拖入“应用程序”文件夹后重新打开，再检查更新。"
                .to_string(),
        );
    }
    None
}

#[cfg(target_os = "macos")]
fn update_install_blocker() -> Option<String> {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return Some(
            "无法确定当前应用的安装位置，请将应用移动到“应用程序”文件夹后重新打开，再检查更新。"
                .to_string(),
        ),
    };
    let bundle = executable
        .parent()
        .and_then(|macos_dir| macos_dir.parent())
        .and_then(|contents_dir| contents_dir.parent())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("app"))
        .map(|path| path.to_path_buf());
    let Some(bundle) = bundle else {
        return Some(
            "当前应用不是从标准 macOS 应用包运行，无法自动更新。请安装正式安装包后再试。"
                .to_string(),
        );
    };
    install_blocker_for_bundle(&bundle)
}

#[cfg(not(target_os = "macos"))]
fn update_install_blocker() -> Option<String> {
    None
}

fn resolve_update_endpoint(endpoint: Option<String>) -> Result<tauri::Url, String> {
    let raw = endpoint
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_UPDATE_ENDPOINT.to_string());
    tauri::Url::parse(&raw).map_err(|e| format!("更新源地址无效：{e}"))
}

#[tauri::command]
pub fn get_update_settings(app: AppHandle) -> Result<UpdateSettings, String> {
    let path = settings_path(&app)?;
    if !path.exists() {
        return Ok(UpdateSettings::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读取更新设置失败：{e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("解析更新设置失败：{e}"))
}

#[tauri::command]
pub fn save_update_settings(app: AppHandle, settings: UpdateSettings) -> Result<(), String> {
    let path = settings_path(&app)?;
    let content =
        serde_json::to_string_pretty(&settings).map_err(|e| format!("序列化更新设置失败：{e}"))?;
    fs::write(&path, content).map_err(|e| format!("保存更新设置失败：{e}"))
}

#[tauri::command]
pub async fn check_app_update(
    app: AppHandle,
    pending_update: State<'_, PendingUpdate>,
    endpoint: Option<String>,
    allow_insecure: Option<bool>,
) -> Result<Option<UpdateMetadata>, String> {
    let url = resolve_update_endpoint(endpoint)?;
    if url.scheme() != "https" && !allow_insecure.unwrap_or(false) {
        return Err(
            "更新源必须使用 HTTPS。内网 HTTP 镜像请先勾选允许 HTTP 更新源。".to_string(),
        );
    }

    let update = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| friendly_update_error(e.to_string()))?;

    let metadata = update.as_ref().map(|update| UpdateMetadata {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        date: update.date.map(|date| date.unix_timestamp()),
        body: update.body.clone(),
        install_blocker: update_install_blocker(),
    });

    *pending_update.0.lock().unwrap() = update;
    Ok(metadata)
}

#[tauri::command]
pub async fn install_app_update(
    app: AppHandle,
    pending_update: State<'_, PendingUpdate>,
    on_event: Channel<UpdateProgress>,
) -> Result<(), String> {
    if let Some(reason) = update_install_blocker() {
        return Err(reason);
    }
    let update = pending_update.0.lock().unwrap().take();
    let Some(update) = update else {
        return Err("没有可安装的更新，请先检查更新".to_string());
    };

    let mut started = false;
    update
        .download_and_install(
            |chunk_length, content_length| {
                if !started {
                    let _ = on_event.send(UpdateProgress::Started { content_length });
                    started = true;
                }
                let _ = on_event.send(UpdateProgress::Progress { chunk_length });
            },
            || {
                let _ = on_event.send(UpdateProgress::Finished);
            },
        )
        .await
        .map_err(|e| friendly_install_error(e.to_string()))?;

    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_endpoint_points_to_github_latest_json() {
        let url = resolve_update_endpoint(None).expect("默认更新源应可解析");
        assert_eq!(url.as_str(), DEFAULT_UPDATE_ENDPOINT);
        assert_eq!(url.scheme(), "https");
    }

    #[test]
    fn custom_endpoint_is_trimmed_and_used() {
        let url = resolve_update_endpoint(Some("  https://example.com/latest.json  ".to_string()))
            .expect("自定义更新源应可解析");
        assert_eq!(url.as_str(), "https://example.com/latest.json");
    }

    #[test]
    fn missing_release_error_mentions_github() {
        let message = friendly_update_error("HTTP status client error (404 Not Found)".to_string());
        assert!(message.contains("GitHub Releases"));
    }

    #[test]
    fn empty_endpoint_falls_back_to_github() {
        let url = resolve_update_endpoint(Some("   ".to_string())).expect("空白更新源应回退到默认地址");
        assert_eq!(url.as_str(), DEFAULT_UPDATE_ENDPOINT);
    }

    #[test]
    fn missing_app_bundle_error_is_actionable() {
        let message = friendly_install_error("No such file or directory (os error 2)".to_string());
        assert!(message.contains("应用程序"));
        assert!(message.contains("重新打开"));
    }

    #[test]
    fn permission_error_is_actionable() {
        let message = friendly_install_error("Permission denied (os error 13)".to_string());
        assert!(message.contains("权限"));
        assert!(message.contains("应用程序"));
    }

    #[test]
    fn signature_error_mentions_release() {
        let message = friendly_install_error("signature verification failed".to_string());
        assert!(message.contains("签名"));
        assert!(message.contains("GitHub Releases"));
    }

    #[test]
    fn network_check_error_is_translated() {
        let message = friendly_update_error("error sending request for url".to_string());
        assert!(message.contains("网络"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn missing_bundle_blocks_install() {
        let bundle = std::env::temp_dir().join("deploy-updater-missing.app");
        let reason = install_blocker_for_bundle(&bundle).expect("不存在的应用包应阻止自动安装");
        assert!(reason.contains("不存在"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn temp_dir_bundle_blocks_install() {
        let bundle = std::env::temp_dir().join(format!(
            "deploy-updater-{}.app",
            std::process::id()
        ));
        std::fs::create_dir_all(&bundle).expect("应能创建测试目录");
        let reason = install_blocker_for_bundle(&bundle).expect("临时目录应阻止自动安装");
        std::fs::remove_dir_all(&bundle).ok();
        assert!(reason.contains("临时目录"));
    }
}
