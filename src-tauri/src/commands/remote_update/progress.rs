use super::models::RemoteUpdateProgress;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
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

/// 在长时间等待期间持续上报进度，避免界面停在固定百分比。
pub(super) struct ProgressHeartbeat {
    stop: mpsc::Sender<()>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ProgressHeartbeat {
    pub(super) fn start(
        window: &tauri::Window,
        step: &str,
        start_percent: usize,
        max_percent: usize,
        message: String,
    ) -> Self {
        let (stop, receiver) = mpsc::channel::<()>();
        let window = window.clone();
        let step = step.to_string();
        let handle = std::thread::spawn(move || {
            let started = Instant::now();
            let mut percent = start_percent;
            loop {
                match receiver.recv_timeout(Duration::from_secs(5)) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                percent = (percent + 1).min(max_percent);
                emit_progress(
                    &window,
                    &step,
                    percent,
                    "running",
                    &format!("{message}（已等待 {} 秒）", started.elapsed().as_secs()),
                );
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for ProgressHeartbeat {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
