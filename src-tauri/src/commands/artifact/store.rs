//! 制品源、访问令牌与本机凭据的读写。
//!
//! 令牌内容只落在本机凭据文件里，列表文件只保存名称与时间；所有写入都经过
//! `core::json_store`，因此与 MCP 服务共用同一套原子写入与跨进程锁。

use std::path::{Path, PathBuf};

use tauri::AppHandle;

use super::*;

pub(super) const SECRET_SERVICE: &str = "deploy-desktop-artifact";
pub(super) fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    crate::core::config::app_config_dir(app)
}

pub(super) fn sources_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join("artifact-sources.json"))
}

pub(super) fn load_sources(app: &AppHandle) -> Result<Vec<ArtifactSource>, String> {
    let path = sources_path(app)?;
    let file = crate::core::json_store::load::<SourceFile>(&path, "制品源配置")?;
    Ok(file.map(|file| file.sources).unwrap_or_default())
}

pub(super) fn store_sources(app: &AppHandle, sources: &[ArtifactSource]) -> Result<(), String> {
    let path = sources_path(app)?;
    crate::core::json_store::save(
        &path,
        &SourceFile {
            sources: sources.to_vec(),
        },
        "制品源配置",
        crate::core::atomic_file::FilePermission::Default,
    )
}

pub(super) fn secrets_file(dir: &Path) -> PathBuf {
    dir.join("artifact-tokens.json")
}

pub(super) fn token_library_path(dir: &Path) -> PathBuf {
    dir.join("artifact-token-library.json")
}

pub(super) fn read_token_library(dir: &Path) -> Result<Vec<ArtifactToken>, String> {
    let path = token_library_path(dir);
    let file = crate::core::json_store::load::<TokenLibraryFile>(&path, "访问令牌列表")?;
    Ok(file.map(|file| file.tokens).unwrap_or_default())
}

pub(super) fn write_token_library(dir: &Path, tokens: &[ArtifactToken]) -> Result<(), String> {
    let path = token_library_path(dir);
    crate::core::json_store::save(
        &path,
        &TokenLibraryFile {
            tokens: tokens.to_vec(),
        },
        "访问令牌列表",
        crate::core::atomic_file::FilePermission::Default,
    )
}

/// 令牌内容在凭据文件中使用独立前缀，与按制品源保存的历史记录区分开。
pub(super) fn token_secret_key(id: &str) -> String {
    format!("token:{id}")
}

pub(super) fn read_secrets(
    dir: &Path,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let path = secrets_file(dir);
    Ok(
        crate::core::json_store::load::<std::collections::BTreeMap<String, String>>(
            &path,
            "本机凭据文件",
        )?
        .unwrap_or_default(),
    )
}

pub(super) fn write_secrets(
    dir: &Path,
    secrets: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let path = secrets_file(dir);
    crate::core::json_store::save(
        &path,
        secrets,
        "本机凭据文件",
        crate::core::atomic_file::FilePermission::OwnerOnly,
    )
}

pub(super) fn keychain_marker(id: &str) -> String {
    format!("__keychain_migrated:{id}")
}

/// 旧版本把令牌放在系统钥匙串里。macOS 上应用未签名时每次读取都会弹授权框，
/// 因此这里只尝试迁移一次，之后统一使用本机凭据文件。
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn legacy_keychain_token(id: &str) -> Option<String> {
    keyring::Entry::new(SECRET_SERVICE, id)
        .ok()?
        .get_password()
        .ok()
        .filter(|value| !value.is_empty())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn delete_legacy_keychain_token(id: &str) {
    if let Ok(entry) = keyring::Entry::new(SECRET_SERVICE, id) {
        let _ = entry.delete_credential();
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(super) fn legacy_keychain_token(_id: &str) -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(super) fn delete_legacy_keychain_token(_id: &str) {}

pub(super) fn secret_set(dir: &Path, id: &str, token: &str) -> Result<(), String> {
    let mut secrets = read_secrets(dir)?;
    secrets.insert(id.to_string(), token.to_string());
    secrets.remove(&keychain_marker(id));
    write_secrets(dir, &secrets)
}

pub(super) fn secret_get(dir: &Path, id: &str) -> Result<Option<String>, String> {
    let secrets = read_secrets(dir)?;
    if let Some(token) = secrets.get(id) {
        return Ok(Some(token.clone()));
    }
    if secrets.contains_key(&keychain_marker(id)) {
        return Ok(None);
    }

    let legacy = legacy_keychain_token(id);
    let mut secrets = read_secrets(dir)?;
    secrets.insert(keychain_marker(id), "1".to_string());
    if let Some(token) = legacy {
        secrets.insert(id.to_string(), token.clone());
        write_secrets(dir, &secrets)?;
        delete_legacy_keychain_token(id);
        return Ok(Some(token));
    }
    write_secrets(dir, &secrets)?;
    Ok(None)
}

pub(super) fn secret_delete(dir: &Path, id: &str) -> Result<(), String> {
    let mut secrets = read_secrets(dir)?;
    secrets.remove(id);
    secrets.remove(&keychain_marker(id));
    delete_legacy_keychain_token(id);
    write_secrets(dir, &secrets)
}

pub(super) fn stored_token(dir: &Path, token_id: &str) -> Result<Option<String>, String> {
    Ok(secret_get(dir, &token_secret_key(token_id))?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

/// 制品源绑定令牌库时只读取该令牌；未绑定时沿用按制品源保存的历史令牌。
pub(super) fn source_token(dir: &Path, source: &ArtifactSource) -> Result<Option<String>, String> {
    let token_id = source.token_id.trim();
    if !token_id.is_empty() {
        return stored_token(dir, token_id);
    }
    Ok(secret_get(dir, &source.id)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

pub(super) fn new_token_id(tokens: &[ArtifactToken]) -> String {
    let mut stamp = now_millis();
    loop {
        let id = format!("tok-{stamp:x}");
        if !tokens.iter().any(|token| token.id == id) {
            return id;
        }
        stamp += 1;
    }
}

pub(super) fn unique_token_name(tokens: &[ArtifactToken], base: &str) -> String {
    if !tokens.iter().any(|token| token.name == base) {
        return base.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base} {index}");
        if !tokens.iter().any(|token| token.name == candidate) {
            return candidate;
        }
        index += 1;
    }
}

/// 旧版本按制品源保存令牌。首次打开令牌列表时统一收进令牌库，
/// 相同内容的令牌只保留一条，制品源改绑到该令牌。
/// 设置页会同时读取令牌与制品源，迁移需要串行执行，避免重复写入。
pub(super) static LEGACY_TOKEN_MIGRATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn migrate_legacy_tokens(app: &AppHandle) -> Result<(), String> {
    let _guard = LEGACY_TOKEN_MIGRATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = config_dir(app)?;
    let mut sources = load_sources(app)?;
    if sources
        .iter()
        .all(|source| !source.token_id.trim().is_empty())
    {
        return Ok(());
    }
    let mut tokens = read_token_library(&dir)?;
    let mut changed = false;

    for source in sources.iter_mut() {
        if !source.token_id.trim().is_empty() {
            continue;
        }
        let value = match secret_get(&dir, &source.id)? {
            Some(value) => value.trim().to_string(),
            None => continue,
        };
        if value.is_empty() {
            continue;
        }

        let mut matched = None;
        for token in tokens.iter() {
            if stored_token(&dir, &token.id)?.as_deref() == Some(value.as_str()) {
                matched = Some(token.id.clone());
                break;
            }
        }

        let token_id = match matched {
            Some(id) => id,
            None => {
                let base = if source.name.trim().is_empty() {
                    "云效访问令牌".to_string()
                } else {
                    format!("{}的令牌", source.name.trim())
                };
                let now = now_millis() as u64;
                let id = new_token_id(&tokens);
                secret_set(&dir, &token_secret_key(&id), &value)?;
                tokens.push(ArtifactToken {
                    id: id.clone(),
                    name: unique_token_name(&tokens, &base),
                    created_at: now,
                    updated_at: now,
                });
                id
            }
        };

        source.token_id = token_id;
        source.has_token = true;
        secret_delete(&dir, &source.id)?;
        changed = true;
    }

    if changed {
        write_token_library(&dir, &tokens)?;
        store_sources(app, &sources)?;
    }
    Ok(())
}

pub(super) fn token_view(
    dir: &Path,
    token: &ArtifactToken,
    sources: &[ArtifactSource],
) -> Result<ArtifactTokenView, String> {
    Ok(ArtifactTokenView {
        id: token.id.clone(),
        name: token.name.clone(),
        created_at: token.created_at,
        updated_at: token.updated_at,
        has_token: stored_token(dir, &token.id)?.is_some(),
        in_use: sources
            .iter()
            .filter(|source| source.token_id.trim() == token.id)
            .count(),
    })
}
