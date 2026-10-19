use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::UNIX_EPOCH;

use tauri::Emitter;

use super::remote_ssh::{
    explain_ssh_failure, resolve_existing_key, run_ssh_script, shell_quote, ssh_base_command,
};

const IMAGE_EXTENSIONS: [&str; 3] = ["tar", "tgz", "tar.gz"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageFileInfo {
    pub name: String,
    pub size_bytes: u64,
    pub modified_ms: u64,
    pub image_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageExpectation {
    pub file_name: String,
    pub image_ref: String,
    pub file_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageBundleScan {
    pub input_dir: String,
    pub images_dir: String,
    pub exists: bool,
    pub files: Vec<ImageFileInfo>,
    pub total_bytes: u64,
    pub expectations: Vec<ImageExpectation>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteImageBundleStatus {
    pub bundle_dir: String,
    pub images_dir: String,
    pub exists: bool,
    pub files: Vec<ImageFileInfo>,
    pub total_bytes: u64,
    pub docker_available: bool,
    pub docker_running: bool,
    pub docker_images: Vec<String>,
    pub expectations: Vec<ImageExpectation>,
    pub missing_images: Vec<String>,
    pub free_kb: u64,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageManagerProgress {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub file_name: String,
    pub percent: usize,
    pub message: String,
    pub finished: bool,
}

#[tauri::command]
pub fn scan_local_image_bundle(dir: String) -> Result<ImageBundleScan, String> {
    let raw = dir.trim();
    if raw.is_empty() {
        return Err("请填写本地镜像包目录".to_string());
    }
    let (input, images_dir) = resolve_images_dir(raw);
    scan_images_dir(&input, &images_dir)
}

#[tauri::command]
pub fn inspect_remote_image_bundle(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    bundle_dir: String,
) -> Result<RemoteImageBundleStatus, String> {
    let key = resolve_existing_key(key_path, &host)?;
    inspect_remote_image_bundle_inner(&host, port, &user, key.as_deref(), &bundle_dir)
}

#[tauri::command]
pub async fn upload_remote_image_bundle(
    window: tauri::Window,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    local_dir: String,
    remote_bundle_dir: String,
    files: Vec<String>,
    overwrite: bool,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        upload_images_blocking(
            &window,
            &host,
            port,
            &user,
            key_path,
            &local_dir,
            &remote_bundle_dir,
            files,
            overwrite,
        )
    })
    .await
    .map_err(|e| format!("上传镜像包任务失败: {e}"))?
}

#[tauri::command]
pub async fn import_remote_image_bundle(
    window: tauri::Window,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    remote_bundle_dir: String,
    files: Vec<String>,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        import_images_blocking(
            &window,
            &host,
            port,
            &user,
            key_path,
            &remote_bundle_dir,
            files,
        )
    })
    .await
    .map_err(|e| format!("导入镜像任务失败: {e}"))?
}

#[tauri::command]
pub fn remove_remote_image_files(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    remote_bundle_dir: String,
    files: Vec<String>,
) -> Result<String, String> {
    if files.is_empty() {
        return Err("请选择要删除的镜像包文件".to_string());
    }
    let key = resolve_existing_key(key_path, &host)?;
    let images_dir = remote_images_dir(&remote_bundle_dir);
    let mut script = String::from("set -e\n");
    let mut removed = Vec::new();
    for name in &files {
        let safe = safe_file_name(name)?;
        script.push_str(&format!(
            "rm -f -- {}\necho \"REMOVED|{}\"\n",
            shell_quote(&format!("{images_dir}/{safe}")),
            safe
        ));
        removed.push(safe);
    }
    let output = run_ssh_script(&host, port, &user, key.as_deref(), &script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "删除目标机镜像包失败: {}",
            explain_ssh_failure(&stderr, key.as_deref())
        ));
    }
    Ok(format!("已删除目标机镜像包: {}", removed.join("、")))
}

fn resolve_images_dir(raw: &str) -> (PathBuf, PathBuf) {
    let input = PathBuf::from(raw);
    if input.file_name().map(|name| name == "images").unwrap_or(false) && input.is_dir() {
        return (input.clone(), input);
    }
    let nested = input.join("images");
    if nested.is_dir() {
        return (input, nested);
    }
    if input.is_dir() && directory_has_images(&input) {
        return (input.clone(), input);
    }
    (input.clone(), nested)
}

fn directory_has_images(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .path()
                    .is_file()
                    .then(|| entry.path())
                    .map(|path| is_image_file(&path))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn is_image_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    IMAGE_EXTENSIONS
        .iter()
        .any(|ext| name.ends_with(&format!(".{ext}")))
}

fn scan_images_dir(input: &Path, images_dir: &Path) -> Result<ImageBundleScan, String> {
    let expectations = expectations_for(images_dir);
    if !images_dir.is_dir() {
        return Ok(ImageBundleScan {
            input_dir: input.to_string_lossy().to_string(),
            images_dir: images_dir.to_string_lossy().to_string(),
            exists: false,
            files: Vec::new(),
            total_bytes: 0,
            expectations,
            message: format!("镜像目录不存在: {}", images_dir.display()),
        });
    }

    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    if let Ok(entries) = fs::read_dir(images_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !is_image_file(&path) {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(value) => value,
                Err(_) => continue,
            };
            let name = entry.file_name().to_string_lossy().to_string();
            total_bytes += metadata.len();
            files.push(ImageFileInfo {
                image_ref: image_ref_for(&name),
                name,
                size_bytes: metadata.len(),
                modified_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0),
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));

    let message = if files.is_empty() {
        format!("镜像目录中没有 .tar 镜像文件: {}", images_dir.display())
    } else {
        format!("共 {} 个镜像包文件", files.len())
    };

    Ok(ImageBundleScan {
        input_dir: input.to_string_lossy().to_string(),
        images_dir: images_dir.to_string_lossy().to_string(),
        exists: true,
        files,
        total_bytes,
        expectations,
        message,
    })
}

fn expectations_for(images_dir: &Path) -> Vec<ImageExpectation> {
    super::profile::deployment_profile()
        .images
        .iter()
        .map(|image| ImageExpectation {
            file_name: image.file.clone(),
            image_ref: image.image_ref.clone(),
            file_present: images_dir.join(&image.file).is_file(),
        })
        .collect()
}

fn image_ref_for(file_name: &str) -> Option<String> {
    super::profile::deployment_profile()
        .images
        .iter()
        .find(|image| image.file == file_name)
        .map(|image| image.image_ref.clone())
}

fn remote_images_dir(bundle_dir: &str) -> String {
    format!("{}/images", bundle_dir.trim().trim_end_matches('/'))
}

fn safe_file_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed == "."
        || trimmed == ".."
    {
        return Err(format!("非法的镜像包文件名: {name}"));
    }
    Ok(trimmed.to_string())
}

fn inspect_remote_image_bundle_inner(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    bundle_dir: &str,
) -> Result<RemoteImageBundleStatus, String> {
    let bundle_dir = bundle_dir.trim().trim_end_matches('/').to_string();
    let images_dir = remote_images_dir(&bundle_dir);
    let script = format!(
        r#"set -e
IMAGES={images}
if [ -d "$IMAGES" ]; then echo "EXISTS=yes"; else echo "EXISTS=no"; fi
DF_PATH="$IMAGES"
while [ ! -d "$DF_PATH" ] && [ "$DF_PATH" != "/" ]; do DF_PATH=$(dirname "$DF_PATH"); done
echo "FREE_KB=$(df -Pk "$DF_PATH" 2>/dev/null | awk 'NR==2{{print $4}}')"
if command -v docker >/dev/null 2>&1; then echo "DOCKER_AVAILABLE=yes"; else echo "DOCKER_AVAILABLE=no"; fi
if docker info >/dev/null 2>&1; then echo "DOCKER_RUNNING=yes"; else echo "DOCKER_RUNNING=no"; fi
echo "---FILES---"
if [ -d "$IMAGES" ]; then
  find "$IMAGES" -maxdepth 1 -type f \( -name '*.tar' -o -name '*.tgz' -o -name '*.tar.gz' \) -print | sort | while IFS= read -r f; do
    printf 'FILE|%s|%s|%s\n' "$(basename "$f")" "$(stat -c '%s' "$f")" "$(stat -c '%Y' "$f")"
  done
fi
echo "---IMAGES---"
if docker info >/dev/null 2>&1; then
  docker image ls --format '{{{{.Repository}}}}:{{{{.Tag}}}}' 2>/dev/null | sort | sed 's/^/IMAGE|/'
fi
"#,
        images = shell_quote(&images_dir)
    );

    let output = run_ssh_script(host, port, user, key, &script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "检测目标机镜像包失败: {}",
            explain_ssh_failure(&stderr, key)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let mut exists = false;
    let mut free_kb = 0u64;
    let mut docker_available = false;
    let mut docker_running = false;
    let mut files = Vec::new();
    let mut docker_images = Vec::new();
    let mut section = String::new();

    for line in stdout.lines() {
        let line = line.trim_end();
        if line == "---FILES---" {
            section = "files".to_string();
            continue;
        }
        if line == "---IMAGES---" {
            section = "images".to_string();
            continue;
        }
        if let Some(value) = line.strip_prefix("EXISTS=") {
            exists = value == "yes";
        } else if let Some(value) = line.strip_prefix("FREE_KB=") {
            free_kb = value.trim().parse::<u64>().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("DOCKER_AVAILABLE=") {
            docker_available = value == "yes";
        } else if let Some(value) = line.strip_prefix("DOCKER_RUNNING=") {
            docker_running = value == "yes";
        } else if section == "files" {
            if let Some(rest) = line.strip_prefix("FILE|") {
                let parts: Vec<&str> = rest.split('|').collect();
                if parts.len() >= 3 {
                    files.push(ImageFileInfo {
                        name: parts[0].to_string(),
                        size_bytes: parts[1].parse::<u64>().unwrap_or(0),
                        modified_ms: parts[2]
                            .parse::<u64>()
                            .map(|secs| secs.saturating_mul(1000))
                            .unwrap_or(0),
                        image_ref: image_ref_for(parts[0]),
                    });
                }
            }
        } else if section == "images" {
            if let Some(value) = line.strip_prefix("IMAGE|") {
                let value = value.trim();
                if !value.is_empty() && !value.contains("<none>") {
                    docker_images.push(value.to_string());
                }
            }
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    let total_bytes = files.iter().map(|file| file.size_bytes).sum();

    let expectations = super::profile::deployment_profile()
        .images
        .iter()
        .map(|image| ImageExpectation {
            file_name: image.file.clone(),
            image_ref: image.image_ref.clone(),
            file_present: files.iter().any(|file| file.name == image.file),
        })
        .collect();

    let missing_images: Vec<String> = super::profile::deployment_profile()
        .images
        .iter()
        .map(|image| image.image_ref.clone())
        .filter(|image_ref| !docker_images.iter().any(|image| image == image_ref))
        .collect();

    let mut issues = Vec::new();
    if !exists {
        issues.push(format!("目标机镜像目录不存在: {images_dir}"));
    }
    if !docker_available {
        issues.push("目标机未安装 Docker".to_string());
    } else if !docker_running {
        issues.push("Docker 守护进程未运行".to_string());
    }
    if exists && files.is_empty() {
        issues.push("目标机镜像目录中没有 .tar 镜像文件".to_string());
    }
    if docker_running && !missing_images.is_empty() {
        issues.push(format!("{} 个镜像尚未导入 Docker", missing_images.len()));
    }

    Ok(RemoteImageBundleStatus {
        bundle_dir,
        images_dir,
        exists,
        files,
        total_bytes,
        docker_available,
        docker_running,
        docker_images,
        expectations,
        missing_images,
        free_kb,
        issues,
    })
}

fn upload_images_blocking(
    window: &tauri::Window,
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<String>,
    local_dir: &str,
    remote_bundle_dir: &str,
    files: Vec<String>,
    overwrite: bool,
) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Err("请选择要上传的镜像包文件".to_string());
    }
    let key = resolve_existing_key(key_path, host)?;
    let (input, images_dir) = resolve_images_dir(local_dir.trim());
    let scan = scan_images_dir(&input, &images_dir)?;
    if !scan.exists {
        return Err(format!("本地镜像目录不存在: {}", images_dir.display()));
    }

    let remote_dir = remote_images_dir(remote_bundle_dir);
    let mkdir = run_ssh_script(host, port, user, key.as_deref(), &format!("mkdir -p {}", shell_quote(&remote_dir)))?;
    if !mkdir.status.success() {
        let stderr = String::from_utf8_lossy(&mkdir.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key.as_deref()));
    }

    let remote_sizes = remote_image_sizes(host, port, user, key.as_deref(), &remote_dir)?;
    let total = files.len();
    let mut messages = Vec::new();

    for (index, raw_name) in files.iter().enumerate() {
        let name = safe_file_name(raw_name)?;
        let local_path = images_dir.join(&name);
        if !local_path.is_file() {
            return Err(format!("本地镜像包文件不存在: {}", local_path.display()));
        }
        let size = fs::metadata(&local_path)
            .map(|meta| meta.len())
            .map_err(|e| format!("读取本地文件失败: {e}"))?;

        if !overwrite {
            if let Some(remote_size) = remote_sizes.get(&name) {
                if *remote_size == size {
                    let message = format!("{name} 已存在且大小一致，跳过上传");
                    messages.push(message.clone());
                    emit_progress(window, "upload", index + 1, total, &name, 100, &message, false);
                    continue;
                }
            }
        }

        upload_one_image(
            window,
            host,
            port,
            user,
            key.as_deref(),
            &local_path,
            &remote_dir,
            &name,
            index + 1,
            total,
        )?;
        messages.push(format!("{name} 上传完成（{:.1} MB）", size as f64 / 1024.0 / 1024.0));
    }

    emit_progress(
        window,
        "done",
        total,
        total,
        "ALL",
        100,
        "镜像包上传完成",
        true,
    );
    Ok(messages)
}

fn remote_image_sizes(
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    remote_dir: &str,
) -> Result<std::collections::HashMap<String, u64>, String> {
    let script = format!(
        "set -e\nDIR={}\nif [ -d \"$DIR\" ]; then\n  find \"$DIR\" -maxdepth 1 -type f -print | sort | while IFS= read -r f; do\n    printf 'FILE|%s|%s\\n' \"$(basename \"$f\")\" \"$(stat -c '%s' \"$f\")\"\n  done\nfi\n",
        shell_quote(remote_dir)
    );
    let output = run_ssh_script(host, port, user, key, &script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key));
    }
    let mut sizes = std::collections::HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(rest) = line.strip_prefix("FILE|") {
            let parts: Vec<&str> = rest.split('|').collect();
            if parts.len() >= 2 {
                sizes.insert(parts[0].to_string(), parts[1].parse::<u64>().unwrap_or(0));
            }
        }
    }
    Ok(sizes)
}

#[allow(clippy::too_many_arguments)]
fn upload_one_image(
    window: &tauri::Window,
    host: &str,
    port: u16,
    user: &str,
    key: Option<&str>,
    local: &Path,
    remote_dir: &str,
    name: &str,
    index: usize,
    total: usize,
) -> Result<(), String> {
    let size = fs::metadata(local)
        .map(|meta| meta.len())
        .map_err(|e| format!("读取本地文件失败: {e}"))?;
    let temp = format!("{remote_dir}/{name}.part");
    let mut child = ssh_base_command(host, port, user, key)
        .arg(format!("cat > {}", shell_quote(&temp)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动镜像上传失败: {e}"))?;

    let mut file = File::open(local).map_err(|e| format!("打开本地镜像包失败: {e}"))?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut written = 0u64;
    let mut last_percent = usize::MAX;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "无法打开 SSH 标准输入".to_string())?;
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|e| format!("读取本地镜像包失败: {e}"))?;
            if read == 0 {
                break;
            }
            stdin
                .write_all(&buffer[..read])
                .map_err(|e| format!("写入远程镜像包失败: {e}"))?;
            written += read as u64;
            let percent = if size > 0 {
                ((written * 100) / size).min(100) as usize
            } else {
                100
            };
            if percent != last_percent {
                last_percent = percent;
                emit_progress(
                    window,
                    "upload",
                    index,
                    total,
                    name,
                    percent,
                    &format!("正在上传 {name}（{percent}%）"),
                    false,
                );
            }
        }
    }
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待镜像上传完成失败: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let _ = run_ssh_script(host, port, user, key, &format!("rm -f {}", shell_quote(&temp)));
        return Err(format!(
            "上传镜像包 {name} 失败: {}",
            explain_ssh_failure(&stderr, key)
        ));
    }

    let final_path = format!("{remote_dir}/{name}");
    let swap = format!(
        "mv -f {} {}",
        shell_quote(&temp),
        shell_quote(&final_path)
    );
    let swap_output = run_ssh_script(host, port, user, key, &swap)?;
    if !swap_output.status.success() {
        let stderr = String::from_utf8_lossy(&swap_output.stderr).trim().to_string();
        return Err(format!(
            "替换远程镜像包 {name} 失败: {}",
            explain_ssh_failure(&stderr, key)
        ));
    }
    Ok(())
}

fn import_images_blocking(
    window: &tauri::Window,
    host: &str,
    port: u16,
    user: &str,
    key_path: Option<String>,
    remote_bundle_dir: &str,
    files: Vec<String>,
) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Err("请选择要导入的镜像包文件".to_string());
    }
    let key = resolve_existing_key(key_path, host)?;
    let remote_dir = remote_images_dir(remote_bundle_dir);
    let total = files.len();
    let mut loaded = Vec::new();

    for (index, raw_name) in files.iter().enumerate() {
        let name = safe_file_name(raw_name)?;
        let remote_path = format!("{remote_dir}/{name}");
        let exists = run_ssh_script(
            host,
            port,
            user,
            key.as_deref(),
            &format!("[ -f {} ] && echo yes || echo no", shell_quote(&remote_path)),
        )?;
        if String::from_utf8_lossy(&exists.stdout).trim() != "yes" {
            return Err(format!("目标机镜像包文件不存在: {remote_path}"));
        }

        emit_progress(
            window,
            "import",
            index + 1,
            total,
            &name,
            0,
            &format!("正在导入镜像 [{}/{}] {name}", index + 1, total),
            false,
        );
        let output = run_ssh_script(
            host,
            port,
            user,
            key.as_deref(),
            &format!("set -e\ndocker load -i {}\n", shell_quote(&remote_path)),
        )?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("导入镜像 {name} 失败: {stderr}"));
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        loaded.push(format!("{name}: {stdout}"));
        emit_progress(
            window,
            "import",
            index + 1,
            total,
            &name,
            100,
            &format!("{name} 已导入"),
            false,
        );
    }

    emit_progress(
        window,
        "done",
        total,
        total,
        "ALL",
        100,
        "镜像导入完成",
        true,
    );
    Ok(loaded)
}

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    window: &tauri::Window,
    phase: &str,
    current: usize,
    total: usize,
    file_name: &str,
    percent: usize,
    message: &str,
    finished: bool,
) {
    let _ = window.emit(
        "image-manager-progress",
        ImageManagerProgress {
            phase: phase.to_string(),
            current,
            total,
            file_name: file_name.to_string(),
            percent,
            message: message.to_string(),
            finished,
        },
    );
}
