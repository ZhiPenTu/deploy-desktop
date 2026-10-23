//! Bounded process execution for SSH scripts and binary uploads.
//! All pipes are drained concurrently; timeout covers input, output and process exit.
use std::io;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Child;

pub type InputProgress = Box<dyn FnMut(u64) + Send>;

pub enum Input {
    Bytes(Vec<u8>),
    File {
        path: PathBuf,
        progress: Option<InputProgress>,
    },
    Producer(Command),
}

#[derive(Clone, Copy)]
pub struct Options {
    pub timeout: Duration,
    pub max_output: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            max_output: 4 * 1024 * 1024,
        }
    }
}

struct Captured {
    bytes: Vec<u8>,
    truncated: bool,
}
async fn capture(mut reader: impl AsyncRead + Unpin, limit: usize) -> io::Result<Captured> {
    let mut bytes = Vec::new();
    let mut buffer = vec![0; 16384];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let keep = count.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..keep]);
        truncated |= keep < count;
    }
    Ok(Captured { bytes, truncated })
}

fn spawn(mut command: Command) -> Result<Child, String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    command
        .spawn()
        .map_err(|error| format!("启动执行进程失败: {error}"))
}

async fn stop(child: &mut Child, id: Option<u32>) {
    // Also close inherited pipes held by local helper descendants after the parent exits.
    #[cfg(unix)]
    if let Some(id) = id {
        unsafe {
            libc::kill(-(id as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = id;
    let _ = child.start_kill();
    let _ = child.wait().await;
}

pub fn run(command: Command, input: Input, options: Options) -> Result<Output, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("无法启动执行任务: {error}"))?
        .block_on(execute(command, input, options))
}

async fn execute(command: Command, input: Input, options: Options) -> Result<Output, String> {
    let mut child = spawn(command)?;
    let id = child.id();
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (mut producer, input) = match input {
        Input::Producer(command) => match spawn(command) {
            Ok(mut producer) => {
                drop(producer.stdin.take());
                (Some(producer), Input::Bytes(vec![]))
            }
            Err(error) => {
                stop(&mut child, id).await;
                return Err(error);
            }
        },
        input => (None, input),
    };
    let producer_id = producer.as_ref().and_then(Child::id);
    let transfer = async {
        let write = feed(stdin, input, &mut producer, options.max_output);
        let (write, stdout, stderr, status) = tokio::join!(
            write,
            capture(stdout, options.max_output),
            capture(stderr, options.max_output),
            child.wait()
        );
        let status = status.map_err(|e| format!("等待执行结果失败: {e}"))?;
        let stdout = stdout.map_err(|e| format!("读取执行结果失败: {e}"))?;
        let stderr = stderr.map_err(|e| format!("读取执行详情失败: {e}"))?;
        if !status.success() {
            return Ok(Output {
                status,
                stdout: stdout.bytes,
                stderr: stderr.bytes,
            });
        }
        let producer_truncated = write?;
        if stdout.truncated || stderr.truncated || producer_truncated {
            return Err("执行输出超过限制，无法确认完整结果，请检查目标状态与日志".to_string());
        }
        Ok(Output {
            status,
            stdout: stdout.bytes,
            stderr: stderr.bytes,
        })
    };
    match tokio::time::timeout(options.timeout, transfer).await {
        Ok(result) => result,
        Err(_) => {
            stop(&mut child, id).await;
            if let Some(producer) = producer.as_mut() {
                stop(producer, producer_id).await;
            }
            Err("执行超时，已停止本机传输进程；请检查目标状态后重试".to_string())
        }
    }
}

async fn feed(
    mut stdin: tokio::process::ChildStdin,
    input: Input,
    producer: &mut Option<Child>,
    limit: usize,
) -> Result<bool, String> {
    let truncated = if let Some(producer) = producer.as_mut() {
        let mut source = producer.stdout.take().unwrap();
        let errors = producer.stderr.take().unwrap();
        let id = producer.id();
        let transfer = async {
            let copy = tokio::io::copy(&mut source, &mut stdin).await;
            if copy.is_err() {
                stop(producer, id).await;
            }
            (copy, producer.wait().await)
        };
        let ((copy, status), errors) = tokio::join!(transfer, capture(errors, limit));
        let status = status.map_err(|e| format!("等待打包结束失败: {e}"))?;
        let errors = errors.map_err(|e| format!("读取打包结果失败: {e}"))?;
        if !status.success() {
            return Err(format!(
                "打包失败（退出码 {:?}）：{}",
                status.code(),
                String::from_utf8_lossy(&errors.bytes)
            ));
        }
        copy.map_err(|e| format!("传输目录失败: {e}"))?;
        errors.truncated
    } else {
        match input {
            Input::Bytes(bytes) => stdin
                .write_all(&bytes)
                .await
                .map_err(|e| format!("写入执行输入失败: {e}"))?,
            Input::File { path, mut progress } => {
                let mut file = tokio::fs::File::open(path)
                    .await
                    .map_err(|e| format!("打开上传文件失败: {e}"))?;
                let mut buffer = vec![0; 128 * 1024];
                let mut written = 0;
                loop {
                    let count = file
                        .read(&mut buffer)
                        .await
                        .map_err(|e| format!("读取上传文件失败: {e}"))?;
                    if count == 0 {
                        break;
                    }
                    stdin
                        .write_all(&buffer[..count])
                        .await
                        .map_err(|e| format!("传输文件失败: {e}"))?;
                    written += count as u64;
                    if let Some(progress) = progress.as_mut() {
                        progress(written);
                    }
                }
            }
            Input::Producer(_) => unreachable!(),
        }
        false
    };
    stdin
        .shutdown()
        .await
        .map_err(|e| format!("结束输入失败: {e}"))?;
    Ok(truncated)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn bash(script: &str) -> Command {
        let mut c = Command::new("bash");
        c.args(["-c", script]);
        c
    }
    #[test]
    fn drains_output_while_writing_large_input() {
        let output = run(
            bash("head -c 200000 /dev/zero >&2; cat"),
            Input::Bytes(vec![b'x'; 200000]),
            Options::default(),
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 200000);
        assert_eq!(output.stderr.len(), 200000);
    }
    #[test]
    fn early_exit_does_not_hide_remote_error() {
        let output = run(
            bash("echo denied >&2; exit 23"),
            Input::Bytes(vec![0; 1_000_000]),
            Options::default(),
        )
        .unwrap();
        assert_eq!(output.status.code(), Some(23));
        assert!(String::from_utf8_lossy(&output.stderr).contains("denied"));
    }
    #[test]
    fn producer_failure_prevents_success() {
        let error = run(
            bash("cat >/dev/null"),
            Input::Producer(bash("echo partial; echo broken >&2; exit 7")),
            Options::default(),
        )
        .unwrap_err();
        assert!(error.contains("broken"), "{error}");
    }
    #[test]
    fn timeout_stops_descendants_holding_pipes() {
        let started = std::time::Instant::now();
        let error = run(
            bash("sleep 30 & wait"),
            Input::Bytes(vec![]),
            Options {
                timeout: Duration::from_millis(100),
                ..Options::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("超时"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn file_progress_runs_on_a_small_worker_stack() {
        let path = std::env::temp_dir().join(format!(
            "deploy-process-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, vec![42; 250_000]).unwrap();
        let progress = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = progress.clone();
        let input_path = path.clone();
        let output = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || {
                run(
                    bash("cat"),
                    Input::File {
                        path: input_path,
                        progress: Some(Box::new(move |bytes| recorded.lock().unwrap().push(bytes))),
                    },
                    Options::default(),
                )
            })
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(output.stdout, vec![42; 250_000]);
        let samples = progress.lock().unwrap();
        assert_eq!(samples.last(), Some(&250_000));
        assert!(samples.windows(2).all(|pair| pair[0] < pair[1]));
    }
    #[test]
    fn output_limit_is_not_a_successful_partial_result() {
        let error = run(
            bash("printf 123456789"),
            Input::Bytes(vec![]),
            Options {
                max_output: 4,
                ..Options::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("超过限制"));
    }
}
