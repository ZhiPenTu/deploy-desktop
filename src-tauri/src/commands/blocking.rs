// Process waits must never run on the webview thread or an async runtime worker.
pub(crate) async fn run<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| "后台任务未能完成，请重试。".to_string())?
}

#[cfg(test)]
mod tests {
    #[test]
    fn process_work_runs_on_a_separate_thread_and_preserves_errors() {
        let caller = std::thread::current().id();
        let worker =
            tauri::async_runtime::block_on(super::run(|| Ok(std::thread::current().id()))).unwrap();
        assert_ne!(caller, worker);
        let failure =
            tauri::async_runtime::block_on(super::run(|| Err::<(), _>("连接未完成".to_string())));
        assert_eq!(failure.unwrap_err(), "连接未完成");
    }

    #[cfg(unix)]
    #[test]
    fn stalled_ssh_check_yields_before_the_connection_finishes() {
        use std::{future::Future, net::TcpListener, sync::mpsc, task::Poll, time::Duration};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let (release, hold) = mpsc::channel::<()>();
        let (accepted, connected) = tokio::sync::oneshot::channel();
        let server = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(4) {
                if let Ok((stream, _)) = listener.accept() {
                    let _ = accepted.send(());
                    // No SSH banner: the command cannot finish until this socket is closed.
                    let _ = hold.recv_timeout(Duration::from_secs(4));
                    drop(stream);
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let key = std::env::temp_dir().join(format!("deploy-stalled-ssh-key-{port}"));
        std::fs::write(&key, "invalid test key").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut check = Box::pin(crate::commands::remote_ssh::check_remote_env(
                "127.0.0.1".to_string(),
                port,
                "deploy-test".to_string(),
                Some(key.to_string_lossy().to_string()),
            ));
            let yielded =
                std::future::poll_fn(|cx| Poll::Ready(check.as_mut().poll(cx).is_pending())).await;
            assert!(
                yielded,
                "SSH must yield instead of blocking the caller thread"
            );
            tokio::time::timeout(Duration::from_secs(3), connected)
                .await
                .unwrap()
                .unwrap();
            release.send(()).unwrap();
            assert!(check.await.is_err());
        });
        server.join().unwrap();
        std::fs::remove_file(key).unwrap();
    }
}
