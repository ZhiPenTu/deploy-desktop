use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

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
    let endpoint = endpoint
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "请填写更新源地址".to_string())?;
    let url = tauri::Url::parse(&endpoint).map_err(|e| format!("更新源地址无效：{e}"))?;
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
        .map_err(|e| e.to_string())?;

    let metadata = update.as_ref().map(|update| UpdateMetadata {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        date: update.date.map(|date| date.unix_timestamp()),
        body: update.body.clone(),
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
        .map_err(|e| e.to_string())?;

    app.restart();
}
