//! 制品源与交付包的数据模型；只描述界面与命令之间传递的字段。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactSource {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub org_id: String,
    pub pipeline_id: String,
    pub artifact_name: String,
    pub version: String,
    pub download_url: String,
    pub token_id: String,
    pub has_token: bool,
    pub last_version: String,
    pub last_bundle_dir: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactSourceInput {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub org_id: String,
    pub pipeline_id: String,
    pub artifact_name: String,
    pub version: String,
    pub download_url: String,
    pub token_id: String,
}

/// 可复用的云效个人访问令牌，只保存名称与时间，令牌内容单独存放。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactToken {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactTokenInput {
    pub id: String,
    pub name: String,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactTokenView {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub has_token: bool,
    pub in_use: usize,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct TokenLibraryFile {
    pub(super) tokens: Vec<ArtifactToken>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtifactRelease {
    pub run_id: i64,
    pub run_url: String,
    pub run_status: String,
    pub version: String,
    pub commit: String,
    pub commit_short: String,
    pub commit_title: String,
    pub branch: String,
    pub built_at: i64,
    pub artifact_name: String,
    pub repo_id: String,
    pub size_bytes: u64,
    pub md5: String,
    pub download_url: String,
    pub local_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReleasePage {
    pub items: Vec<ArtifactRelease>,
    pub page: u32,
    pub per_page: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub stage: String,
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactBundle {
    pub dir: String,
    pub version: String,
    pub commit: String,
    pub md5: String,
    pub size_bytes: u64,
    pub files: usize,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct SourceFile {
    pub(super) sources: Vec<ArtifactSource>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct CachedReleaseEntry {
    pub(super) signature: String,
    pub(super) cached_at: i64,
    pub(super) has_more: bool,
    pub(super) items: Vec<ArtifactRelease>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct ReleaseCacheFile {
    pub(super) sources: std::collections::BTreeMap<String, CachedReleaseEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBundleRemoval {
    pub dir: String,
    pub freed_bytes: u64,
}
