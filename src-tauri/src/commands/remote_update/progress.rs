use super::models::RemoteUpdateProgress;
use tauri::Emitter;

pub(super) fn emit_progress(
    window: &tauri::Window,
    step: &str,
    percent: usize,
    status: &str,
    message: &str,
) {
    let _ = window.emit(
        "remote-update-progress",
        RemoteUpdateProgress {
            step: step.to_string(),
            percent,
            status: status.to_string(),
            message: message.to_string(),
        },
    );
}
