//! 交付包描述符（deploy.desktop/bundle/v2）。
//!
//! 交付包用 `bundle.json` 描述产物映射、组件、服务、数据库、镜像与部署步骤，
//! 客户端只按声明执行，不再内置任何项目专用的目录约定。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const API_VERSION: &str = "deploy.desktop/bundle/v2";
pub const KIND: &str = "DeploymentBundle";
pub const IMAGE_COMPONENT: &str = "image";
const IGNORED_FILE: &str = ".DS_Store";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleDescriptor {
    pub api_version: String,
    pub kind: String,
    pub metadata: BundleMetadata,
    pub runtime: RuntimeSpec,
    #[serde(default)]
    pub components: Vec<ComponentSpec>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactSpec>,
    #[serde(default)]
    pub images: Vec<ImageSpec>,
    #[serde(default)]
    pub plan: PlanSpec,
    #[serde(default)]
    pub files: Vec<FileSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleMetadata {
    pub product: String,
    #[serde(default)]
    pub display_name: String,
    pub version: String,
    #[serde(default)]
    pub build_number: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub commit_short: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub built_at: String,
    #[serde(default)]
    pub min_tool_version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeSpec {
    #[serde(rename = "type")]
    pub runtime_type: String,
    pub compose: String,
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub container_prefix: String,
    #[serde(default)]
    pub services: Vec<ServiceSpec>,
    #[serde(default)]
    pub database: Option<DatabaseSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceSpec {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub container: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseSpec {
    pub service_key: String,
    pub name: String,
    pub client: String,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactRoot {
    Install,
    Deploy,
}

impl ArtifactRoot {
    pub fn as_str(&self) -> &'static str {
        match self {
            ArtifactRoot::Install => "install",
            ArtifactRoot::Deploy => "deploy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    File,
    Dir,
}

impl ArtifactKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ArtifactKind::File => "file",
            ArtifactKind::Dir => "dir",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactSpec {
    pub key: String,
    #[serde(default)]
    pub label: String,
    pub source: String,
    pub root: ArtifactRoot,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: ArtifactKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentSpec {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub services: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageRole {
    App,
    Base,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageSpec {
    pub name: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub role: ImageRole,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub bundle_file: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanSpec {
    #[serde(default)]
    pub steps: Vec<PlanStepSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanStepSpec {
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub hint: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileSpec {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedArtifact {
    pub key: String,
    pub root: String,
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedRecord {
    pub schema_version: u32,
    pub project_id: String,
    pub product: String,
    pub version: String,
    pub installed_at: String,
    pub artifacts: Vec<DeployedArtifact>,
}

pub const DEPLOYED_RECORD_DIR: &str = ".deploy-desktop";
pub const DEPLOYED_RECORD_FILE: &str = "deployed.json";

#[derive(Debug, Clone)]
pub struct ResolvedArtifact {
    pub key: String,
    pub label: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub kind: ArtifactKind,
}

#[derive(Debug, Clone)]
pub struct Bundle {
    pub root: PathBuf,
    pub descriptor: BundleDescriptor,
}

impl Bundle {
    pub fn load(root: &Path) -> Result<Bundle, String> {
        let canonical = fs::canonicalize(root)
            .map_err(|_| "无法访问交付包目录，请重新选择并解压制品".to_string())?;
        let manifest = canonical.join("bundle.json");
        let text = fs::read_to_string(&manifest).map_err(|_| {
            "交付包缺少 bundle.json，请使用新版流水线重新构建并上传制品".to_string()
        })?;
        let descriptor = match serde_json::from_str::<BundleDescriptor>(&text) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                log::warn!("bundle.json 解析失败: {error}");
                return Err(
                    "交付包信息格式不正确，请使用新版流水线重新构建并上传制品".to_string(),
                );
            }
        };
        let bundle = Bundle {
            root: canonical,
            descriptor,
        };
        bundle.validate_structure()?;
        Ok(bundle)
    }

    pub fn load_optional(root: &Path) -> Option<Bundle> {
        if !root.is_dir() || !root.join("bundle.json").is_file() {
            return None;
        }
        Bundle::load(root).ok()
    }

    pub fn metadata(&self) -> &BundleMetadata {
        &self.descriptor.metadata
    }

    pub fn product(&self) -> &str {
        self.descriptor.metadata.product.trim()
    }

    pub fn version(&self) -> &str {
        self.descriptor.metadata.version.trim()
    }

    /// 交付包内 Compose 清单所在目录（相对交付包根），用于确定 `deploy` 根。
    pub fn deploy_source_rel(&self) -> String {
        let compose = safe_relative(&self.descriptor.runtime.compose, "Compose 清单路径")
            .unwrap_or_default();
        Path::new(&compose)
            .parent()
            .map(|parent| parent.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    }

    pub fn deploy_source_dir(&self) -> PathBuf {
        let rel = self.deploy_source_rel();
        if rel.is_empty() {
            self.root.clone()
        } else {
            self.root.join(rel)
        }
    }

    pub fn component(&self, key: &str) -> Option<&ComponentSpec> {
        self.descriptor.components.iter().find(|item| item.key == key)
    }

    pub fn artifact(&self, key: &str) -> Option<&ArtifactSpec> {
        self.descriptor.artifacts.iter().find(|item| item.key == key)
    }

    /// 按包内来源查找产物；Compose 清单必须有一个对应的 artifact。
    pub fn artifact_for_source(&self, source: &str) -> Option<&ArtifactSpec> {
        let source = source.trim();
        self.descriptor
            .artifacts
            .iter()
            .find(|item| item.source.trim() == source)
    }

    pub fn service(&self, key: &str) -> Option<&ServiceSpec> {
        self.descriptor.runtime.services.iter().find(|item| item.key == key)
    }

    pub fn select_components(&self, keys: &[String]) -> Result<Vec<&ComponentSpec>, String> {
        let mut selected: Vec<&ComponentSpec> = Vec::new();
        for key in keys {
            if key == IMAGE_COMPONENT {
                continue;
            }
            let component = self.component(key).ok_or_else(|| {
                format!("交付包未声明组件 {key}，请重新选择制品或刷新部署计划")
            })?;
            if !selected.iter().any(|item| item.key == component.key) {
                selected.push(component);
            }
        }
        Ok(selected)
    }

    pub fn artifacts_for(&self, components: &[&ComponentSpec]) -> Vec<&ArtifactSpec> {
        let mut result: Vec<&ArtifactSpec> = Vec::new();
        for component in components {
            for key in &component.artifacts {
                if let Some(artifact) = self.artifact(key) {
                    if !result.iter().any(|item| item.key == artifact.key) {
                        result.push(artifact);
                    }
                }
            }
        }
        result
    }

    pub fn resolve_artifacts(
        &self,
        components: &[&ComponentSpec],
        install_dir: &Path,
        deploy_dir: &Path,
    ) -> Result<Vec<ResolvedArtifact>, String> {
        let mut resolved = Vec::new();
        for artifact in self.artifacts_for(components) {
            let source = self.root.join(&artifact.source);
            let base = match artifact.root {
                ArtifactRoot::Install => install_dir.to_path_buf(),
                ArtifactRoot::Deploy => deploy_dir.to_path_buf(),
            };
            let target = base.join(&artifact.path);
            ensure_within(&base, &target, &artifact.key)?;
            resolved.push(ResolvedArtifact {
                key: artifact.key.clone(),
                label: artifact.label.clone(),
                source,
                target,
                kind: artifact.kind,
            });
        }
        Ok(resolved)
    }

    pub fn image_files(&self) -> Vec<PathBuf> {
        self.descriptor
            .images
            .iter()
            .filter(|image| !image.bundle_file.trim().is_empty())
            .map(|image| self.root.join(image.bundle_file.trim()))
            .filter(|path| path.is_file())
            .collect()
    }

    pub fn has_bundle_images(&self) -> bool {
        self.descriptor
            .images
            .iter()
            .any(|image| !image.bundle_file.trim().is_empty())
    }

    pub fn container_prefix(&self) -> &str {
        self.descriptor.runtime.container_prefix.trim()
    }

    pub fn container_name(&self, service_key: &str) -> String {
        if let Some(service) = self.service(service_key) {
            if let Some(container) = service
                .container
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return container.to_string();
            }
        }
        let prefix = self.container_prefix();
        if prefix.is_empty() {
            service_key.to_string()
        } else {
            format!("{prefix}-{service_key}")
        }
    }

    pub fn deployed_record(&self, project_id: &str) -> DeployedRecord {
        DeployedRecord {
            schema_version: 1,
            project_id: project_id.to_string(),
            product: self.product().to_string(),
            version: self.version().to_string(),
            installed_at: chrono_like_now(),
            artifacts: self
                .descriptor
                .artifacts
                .iter()
                .map(|artifact| DeployedArtifact {
                    key: artifact.key.clone(),
                    root: artifact.root.as_str().to_string(),
                    path: artifact.path.clone(),
                    kind: artifact.kind.as_str().to_string(),
                })
                .collect(),
        }
    }

    pub fn ensure_tool_version(&self, tool_version: &str) -> Result<(), String> {
        let required = self.descriptor.metadata.min_tool_version.trim();
        if required.is_empty() {
            return Ok(());
        }
        if compare_versions(tool_version, required) < 0 {
            return Err(format!(
                "该交付包需要部署舱 {required} 或更高版本，当前为 {tool_version}，请先升级客户端"
            ));
        }
        Ok(())
    }

    pub fn ensure_compatible(&self) -> Result<(), String> {
        self.ensure_tool_version(env!("CARGO_PKG_VERSION"))
    }

    fn validate_structure(&self) -> Result<(), String> {
        let descriptor = &self.descriptor;
        if descriptor.api_version != API_VERSION {
            return Err(
                "该交付包使用旧格式，当前版本不再支持。请使用新版流水线重新构建并上传制品。"
                    .to_string(),
            );
        }
        if descriptor.kind != KIND {
            return Err("交付包类型不正确，请使用新版流水线重新构建并上传制品".to_string());
        }
        if descriptor.metadata.product.trim().is_empty()
            || descriptor.metadata.version.trim().is_empty()
        {
            return Err("交付包缺少产品名称或版本号，请重新构建制品".to_string());
        }
        if descriptor.metadata.version.chars().count() > 128 {
            return Err("交付包版本号超过 128 个字符，请重新构建制品".to_string());
        }

        let runtime = &descriptor.runtime;
        if !runtime.runtime_type.eq_ignore_ascii_case("compose") {
            return Err(format!(
                "暂不支持「{}」类型的部署方式，请升级客户端或改用 Compose 交付包",
                runtime.runtime_type.trim()
            ));
        }
        if runtime.container_prefix.contains(['\n', '\r', '\0']) {
            return Err("交付包声明的容器前缀无效，请重新构建制品".to_string());
        }
        let compose = safe_relative(&runtime.compose, "Compose 清单路径")?;

        let mut service_keys: BTreeSet<&str> = BTreeSet::new();
        for service in &runtime.services {
            let key = service.key.trim();
            if key.is_empty() {
                return Err("交付包声明的服务缺少标识，请重新构建制品".to_string());
            }
            if !service_keys.insert(key) {
                return Err(format!("交付包重复声明服务 {key}，请重新构建制品"));
            }
            if service.label.trim().is_empty() {
                return Err(format!("交付包服务 {key} 缺少显示名称，请重新构建制品"));
            }
        }
        if let Some(database) = &runtime.database {
            if database.service_key.trim().is_empty() || database.client.trim().is_empty() {
                return Err("交付包的数据库声明不完整，请重新构建制品".to_string());
            }
            if database.port == 0 {
                return Err("交付包的数据库端口无效，请重新构建制品".to_string());
            }
            if !service_keys.contains(database.service_key.trim()) {
                return Err(format!(
                    "交付包的数据库服务 {} 未在服务清单中声明，请重新构建制品",
                    database.service_key
                ));
            }
        }

        let mut file_paths: BTreeSet<String> = BTreeSet::new();
        for file in &descriptor.files {
            let path = safe_relative(&file.path, "交付包文件路径")?;
            if !file_paths.insert(path.clone()) {
                return Err(format!("交付包重复声明文件 {path}，请重新构建制品"));
            }
            if !is_sha256(&file.sha256) {
                return Err(format!("交付包文件 {path} 的摘要无效，请重新构建制品"));
            }
        }
        if !file_paths.contains(&compose) {
            return Err(format!(
                "交付包未声明 Compose 清单 {compose}，请重新构建制品"
            ));
        }

        let mut ownership: BTreeMap<String, String> = BTreeMap::new();
        let mut artifact_keys: BTreeSet<&str> = BTreeSet::new();
        let mut targets: BTreeSet<(String, String)> = BTreeSet::new();
        for artifact in &descriptor.artifacts {
            let key = artifact.key.trim();
            if key.is_empty() {
                return Err("交付包声明的产物缺少标识，请重新构建制品".to_string());
            }
            if !artifact_keys.insert(key) {
                return Err(format!("交付包重复声明产物 {key}，请重新构建制品"));
            }
            let source = safe_relative(&artifact.source, "产物来源路径")?;
            let target = safe_relative(&artifact.path, "产物部署路径")?;
            if source == "bundle.json" {
                return Err("产物来源不能是 bundle.json，请重新构建制品".to_string());
            }
            if !targets.insert((artifact.root.as_str().to_string(), target.clone())) {
                return Err(format!("交付包重复声明部署位置 {target}，请重新构建制品"));
            }
            let owned: Vec<String> = match artifact.kind {
                ArtifactKind::File => {
                    if !file_paths.contains(&source) {
                        return Err(format!(
                            "产物 {key} 的来源 {source} 不在交付包文件清单中，请重新构建制品"
                        ));
                    }
                    vec![source.clone()]
                }
                ArtifactKind::Dir => {
                    let prefix = format!("{source}/");
                    let owned: Vec<String> = file_paths
                        .iter()
                        .filter(|path| path.starts_with(&prefix))
                        .cloned()
                        .collect();
                    if owned.is_empty() {
                        return Err(format!(
                            "产物 {key} 的目录 {source} 在交付包中为空，请重新构建制品"
                        ));
                    }
                    owned
                }
            };
            for path in owned {
                if let Some(existing) = ownership.insert(path.clone(), key.to_string()) {
                    return Err(format!(
                        "交付包文件 {path} 同时归属于 {existing} 与 {key}，请重新构建制品"
                    ));
                }
            }
        }

        for image in &descriptor.images {
            if image.name.trim().is_empty() || image.reference.trim().is_empty() {
                return Err("交付包声明的镜像缺少名称或引用，请重新构建制品".to_string());
            }
            if image.reference.contains(['\n', '\r', '\0', ' ']) {
                return Err(format!(
                    "交付包镜像 {} 的引用无效，请重新构建制品",
                    image.name
                ));
            }
            if image.bundle_file.trim().is_empty() {
                continue;
            }
            let path = safe_relative(&image.bundle_file, "镜像包路径")?;
            if !file_paths.contains(&path) {
                return Err(format!(
                    "镜像包 {path} 不在交付包文件清单中，请重新构建制品"
                ));
            }
            if let Some(existing) = ownership.insert(path.clone(), format!("image:{}", image.name))
            {
                return Err(format!(
                    "交付包文件 {path} 同时归属于 {existing} 与镜像 {}，请重新构建制品",
                    image.name
                ));
            }
        }

        for path in &file_paths {
            if !ownership.contains_key(path) {
                return Err(format!(
                    "交付包文件 {path} 未归属于任何产物或镜像，请重新构建制品"
                ));
            }
        }

        let mut component_keys: BTreeSet<&str> = BTreeSet::new();
        for component in &descriptor.components {
            let key = component.key.trim();
            if key.is_empty() || key == IMAGE_COMPONENT {
                return Err("交付包声明的组件标识无效，请重新构建制品".to_string());
            }
            if !component_keys.insert(key) {
                return Err(format!("交付包重复声明组件 {key}，请重新构建制品"));
            }
            if component.label.trim().is_empty() {
                return Err(format!("交付包组件 {key} 缺少显示名称，请重新构建制品"));
            }
            for artifact in &component.artifacts {
                if !artifact_keys.contains(artifact.as_str()) {
                    return Err(format!(
                        "组件 {key} 引用了未声明的产物 {artifact}，请重新构建制品"
                    ));
                }
            }
            for service in &component.services {
                if !service_keys.contains(service.as_str()) {
                    return Err(format!(
                        "组件 {key} 引用了未声明的服务 {service}，请重新构建制品"
                    ));
                }
            }
        }

        Ok(())
    }

    /// 校验解包结果：文件清单与实际内容必须完全一致，且摘要匹配。
    pub fn verify_payload(&self) -> Result<(), String> {
        let mut actual: BTreeMap<String, u64> = BTreeMap::new();
        collect_files(&self.root, &self.root, &mut actual)?;

        let declared: BTreeMap<&str, &FileSpec> = self
            .descriptor
            .files
            .iter()
            .map(|file| (file.path.as_str(), file))
            .collect();

        for (path, size) in &actual {
            match declared.get(path.as_str()) {
                Some(spec) => {
                    if spec.size != *size {
                        return Err(format!(
                            "交付包文件大小与清单不一致：{path}，请重新下载制品"
                        ));
                    }
                }
                None => {
                    return Err(format!(
                        "交付包包含未声明的文件：{path}，请使用新版流水线重新构建制品"
                    ))
                }
            }
        }

        for spec in &self.descriptor.files {
            if !actual.contains_key(&spec.path) {
                return Err(format!("交付包缺少文件：{}，请重新下载制品", spec.path));
            }
            let bytes = fs::read(self.root.join(&spec.path))
                .map_err(|_| format!("无法读取交付包文件：{}，请重新下载制品", spec.path))?;
            if sha256_hex(&bytes) != spec.sha256.to_ascii_lowercase() {
                return Err(format!(
                    "交付包文件校验失败：{}，请重新下载制品",
                    spec.path
                ));
            }
        }
        Ok(())
    }
}

fn collect_files(root: &Path, dir: &Path, out: &mut BTreeMap<String, u64>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|_| "无法读取交付包内容，请重新下载制品".to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| "无法读取交付包内容，请重新下载制品".to_string())?;
        if file_type.is_symlink() {
            return Err(format!(
                "交付包包含符号链接：{}，请重新构建制品",
                path.strip_prefix(root).unwrap_or(&path).display()
            ));
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if file_type.is_dir() {
            collect_files(root, &path, out)?;
            continue;
        }
        if name == IGNORED_FILE {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if relative == "bundle.json" {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        out.insert(relative, size);
    }
    Ok(())
}

pub fn safe_relative(value: &str, label: &str) -> Result<String, String> {
    let text = value.trim();
    if text.is_empty() {
        return Err(format!("{label}为空，请重新构建制品"));
    }
    if text.contains('\\') {
        return Err(format!("{label}必须使用 / 分隔：{text}"));
    }
    let candidate = Path::new(text);
    if candidate.is_absolute() {
        return Err(format!("{label}不能是绝对路径：{text}"));
    }
    let mut has_normal = false;
    for part in candidate.components() {
        match part {
            Component::Normal(_) => has_normal = true,
            _ => return Err(format!("{label}包含不合法路径：{text}")),
        }
    }
    if !has_normal {
        return Err(format!("{label}包含不合法路径：{text}"));
    }
    Ok(text.to_string())
}

pub fn ensure_within(base: &Path, target: &Path, label: &str) -> Result<(), String> {
    if !target.starts_with(base) {
        return Err(format!("产物 {label} 的部署路径超出部署目录，请重新构建制品"));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    let text = value.trim();
    text.len() == 64 && text.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn compare_versions(left: &str, right: &str) -> i32 {
    let parse = |value: &str| -> Vec<u32> {
        value
            .trim()
            .trim_start_matches('v')
            .split(['.', '-', '+'])
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect()
    };
    let left = parse(left);
    let right = parse(right);
    for index in 0..left.len().max(right.len()) {
        let a = *left.get(index).unwrap_or(&0);
        let b = *right.get(index).unwrap_or(&0);
        if a != b {
            return if a < b { -1 } else { 1 };
        }
    }
    0
}

pub fn write_deployed_record(install_dir: &Path, record: &DeployedRecord) -> Result<(), String> {
    let dir = install_dir.join(DEPLOYED_RECORD_DIR);
    fs::create_dir_all(&dir).map_err(|_| "无法写入部署记录目录".to_string())?;
    let text = serde_json::to_string_pretty(record)
        .map_err(|_| "无法生成部署记录".to_string())?;
    let target = dir.join(DEPLOYED_RECORD_FILE);
    let temp = dir.join(format!("{DEPLOYED_RECORD_FILE}.tmp"));
    fs::write(&temp, text).map_err(|_| "无法写入部署记录".to_string())?;
    fs::rename(&temp, &target).map_err(|_| "无法保存部署记录".to_string())?;
    Ok(())
}

pub fn read_deployed_record(install_dir: &Path) -> Option<DeployedRecord> {
    let path = install_dir.join(DEPLOYED_RECORD_DIR).join(DEPLOYED_RECORD_FILE);
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    seconds.to_string()
}

/// 校验并归一化更新组件：只接受交付包声明的组件，以及包内含离线镜像时的 `image`。
pub fn normalize_components(bundle: &Bundle, input: Vec<String>) -> Result<Vec<String>, String> {
    let mut result: Vec<String> = Vec::new();
    for item in input {
        let key = item.trim();
        if key.is_empty() {
            continue;
        }
        if key == IMAGE_COMPONENT {
            if !bundle.has_bundle_images() {
                return Err("该交付包未包含离线镜像，无需选择镜像更新".to_string());
            }
            if !result.iter().any(|value| value == key) {
                result.push(key.to_string());
            }
            continue;
        }
        if bundle.component(key).is_none() {
            return Err(format!("交付包未声明组件 {key}，请重新选择制品"));
        }
        if !result.iter().any(|value| value == key) {
            result.push(key.to_string());
        }
    }
    if result.is_empty() {
        return Err("请至少选择一个更新组件".to_string());
    }
    Ok(result)
}

pub fn default_components(bundle: &Bundle) -> Vec<String> {
    let mut keys: Vec<String> = bundle
        .descriptor
        .components
        .iter()
        .map(|component| component.key.clone())
        .collect();
    if bundle.has_bundle_images() {
        keys.push(IMAGE_COMPONENT.to_string());
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "bundle-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// 按目录实际内容计算 files[]，测试只需要声明产物与组件。
    fn files_of(root: &Path) -> Vec<Value> {
        let mut entries = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(current) = stack.pop() {
            let Ok(dir) = fs::read_dir(&current) else {
                continue;
            };
            for entry in dir.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if relative == "bundle.json" {
                    continue;
                }
                let bytes = fs::read(&path).unwrap();
                entries.push(json!({
                    "path": relative,
                    "size": bytes.len(),
                    "sha256": sha256_hex(&bytes),
                }));
            }
        }
        entries.sort_by(|left, right| {
            left["path"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["path"].as_str().unwrap_or_default())
        });
        entries
    }

    fn base_manifest(root: &Path, compose: &str, artifacts: Value, components: Value) -> Value {
        json!({
            "apiVersion": API_VERSION,
            "kind": KIND,
            "metadata": {
                "product": "demo",
                "displayName": "演示产品",
                "version": "12-ab12cd3",
                "buildNumber": "12",
                "commit": "ab12cd3ab12cd3ab12cd3ab12cd3ab12cd3ab12",
                "commitShort": "ab12cd3",
                "branch": "main",
                "builtAt": "2026-10-21T08:00:00Z"
            },
            "runtime": {
                "type": "compose",
                "compose": compose,
                "containerPrefix": "demo",
                "services": [{ "key": "web", "label": "网页", "port": 8080, "protocol": "TCP" }]
            },
            "components": components,
            "artifacts": artifacts,
            "images": [],
            "plan": { "steps": [{ "key": "verify", "title": "验收", "hint": "打开页面确认" }] },
            "files": files_of(root),
        })
    }

    fn write_manifest(root: &Path, manifest: &Value) {
        fs::write(
            root.join("bundle.json"),
            serde_json::to_string_pretty(manifest).unwrap(),
        )
        .unwrap();
    }

    /// 与具体项目无关的目录：包内 app/web → 部署 site，app/bin → install/bin。
    fn generic_bundle() -> PathBuf {
        let root = temp_dir("generic");
        write_file(&root, "app/web/index.html", "<h1>hi</h1>");
        write_file(&root, "app/bin/tool", "binary");
        write_file(&root, "stack/docker-compose.yml", "services:\n  web:\n    image: nginx:1.27\n");
        let manifest = base_manifest(
            &root,
            "stack/docker-compose.yml",
            json!([
                { "key": "web", "label": "网页产物", "source": "app/web", "root": "deploy",
                  "path": "site", "type": "dir" },
                { "key": "bin", "label": "命令行", "source": "app/bin", "root": "install",
                  "path": "bin", "type": "dir" },
                { "key": "compose", "label": "部署配置", "source": "stack/docker-compose.yml",
                  "root": "deploy", "path": "docker-compose.yml", "type": "file" }
            ]),
            json!([
                { "key": "site", "label": "网站", "artifacts": ["web"], "services": ["web"] },
                { "key": "cli", "label": "工具", "artifacts": ["bin"], "services": [] },
                { "key": "config", "label": "部署配置", "artifacts": ["compose"], "services": [] }
            ]),
        );
        write_manifest(&root, &manifest);
        root
    }

    #[test]
    fn generic_bundle_resolves_targets_from_descriptor() {
        let root = generic_bundle();
        let bundle = Bundle::load(&root).unwrap();
        bundle.verify_payload().unwrap();
        assert_eq!(bundle.deploy_source_rel(), "stack");
        assert_eq!(bundle.product(), "demo");
        assert_eq!(bundle.metadata().commit_short, "ab12cd3");

        let keys = vec!["site".to_string(), "cli".to_string()];
        let components = bundle.select_components(&keys).unwrap();
        let install = Path::new("/srv/demo");
        let deploy = install.join(bundle.deploy_source_rel());
        let resolved = bundle.resolve_artifacts(&components, install, &deploy).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].target, Path::new("/srv/demo/stack/site"));
        assert_eq!(resolved[1].target, Path::new("/srv/demo/bin"));
        assert_eq!(default_components(&bundle), vec!["site", "cli", "config"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn java_shaped_bundle_keeps_declared_paths() {
        let root = temp_dir("java");
        write_file(&root, "app/server.jar", "jar");
        write_file(&root, "app/gateway/lib/a.jar", "lib");
        write_file(&root, "deploy/docker-compose.yml", "services:\n  api:\n    image: demo/api:1\n");
        let manifest = base_manifest(
            &root,
            "deploy/docker-compose.yml",
            json!([
                { "key": "server", "source": "app/server.jar", "root": "install",
                  "path": "backend/server/target/server.jar", "type": "file" },
                { "key": "lib", "source": "app/gateway/lib", "root": "install",
                  "path": "backend/gateway/target/lib", "type": "dir" },
                { "key": "compose", "source": "deploy/docker-compose.yml", "root": "deploy",
                  "path": "docker-compose.yml", "type": "file" }
            ]),
            json!([{ "key": "api", "label": "后端", "artifacts": ["server", "lib"], "services": ["web"] }]),
        );
        write_manifest(&root, &manifest);

        let bundle = Bundle::load(&root).unwrap();
        let keys = vec!["api".to_string()];
        let components = bundle.select_components(&keys).unwrap();
        let resolved = bundle
            .resolve_artifacts(&components, Path::new("/opt/app"), Path::new("/opt/app/deploy"))
            .unwrap();
        assert_eq!(
            resolved[0].target,
            Path::new("/opt/app/backend/server/target/server.jar")
        );
        assert_eq!(resolved[1].target, Path::new("/opt/app/backend/gateway/target/lib"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_fields_and_wrong_api_version() {
        let root = generic_bundle();
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(root.join("bundle.json")).unwrap()).unwrap();
        manifest["mode"] = json!("file");
        write_manifest(&root, &manifest);
        let error = Bundle::load(&root).unwrap_err();
        assert!(!error.is_empty());

        let mut manifest: Value = serde_json::from_str(
            &fs::read_to_string(generic_bundle().join("bundle.json")).unwrap(),
        )
        .unwrap();
        manifest["apiVersion"] = json!("deploy.desktop/bundle/v1");
        let other = temp_dir("api-version");
        write_file(&other, "app/web/index.html", "<h1>hi</h1>");
        fs::write(
            other.join("bundle.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let error = Bundle::load(&other).unwrap_err();
        assert!(error.contains("旧格式"), "unexpected: {error}");

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn rejects_absolute_parent_and_duplicate_paths() {
        let root = generic_bundle();
        let source: Value =
            serde_json::from_str(&fs::read_to_string(root.join("bundle.json")).unwrap()).unwrap();

        for (field, value) in [
            ("source", json!("/etc/passwd")),
            ("path", json!("../escape")),
            ("source", json!("app/../app/web")),
        ] {
            let mut manifest = source.clone();
            manifest["artifacts"][0][field] = value;
            write_manifest(&root, &manifest);
            assert!(Bundle::load(&root).is_err());
        }

        let mut manifest = source.clone();
        manifest["artifacts"][1] = json!({
            "key": "bin", "source": "app/bin", "root": "deploy", "path": "site", "type": "dir"
        });
        write_manifest(&root, &manifest);
        let error = Bundle::load(&root).unwrap_err();
        assert!(error.contains("重复声明部署位置"), "unexpected: {error}");

        let mut manifest = source.clone();
        manifest["artifacts"][1]["key"] = json!("web");
        write_manifest(&root, &manifest);
        assert!(Bundle::load(&root).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_uncovered_files_and_unknown_component_references() {
        let root = generic_bundle();
        write_file(&root, "app/extra.txt", "not declared");
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(root.join("bundle.json")).unwrap()).unwrap();
        manifest["files"] = json!(files_of(&root));
        write_manifest(&root, &manifest);
        let error = Bundle::load(&root).unwrap_err();
        assert!(error.contains("未归属于"), "unexpected: {error}");

        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(root.join("bundle.json")).unwrap()).unwrap();
        manifest["files"] = json!(files_of(&root));
        manifest["components"][0]["artifacts"] = json!(["missing"]);
        write_manifest(&root, &manifest);
        assert!(Bundle::load(&root).is_err());

        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(root.join("bundle.json")).unwrap()).unwrap();
        manifest["files"] = json!(files_of(&root));
        manifest["components"][0]["services"] = json!(["unknown"]);
        write_manifest(&root, &manifest);
        assert!(Bundle::load(&root).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verify_payload_rejects_digest_mismatch_and_missing_files() {
        let root = generic_bundle();
        let bundle = Bundle::load(&root).unwrap();
        bundle.verify_payload().unwrap();
        write_file(&root, "app/web/index.html", "<h1>changed</h1>");
        let error = bundle.verify_payload().unwrap_err();
        assert!(error.contains("大小") || error.contains("校验"), "unexpected: {error}");

        let other = generic_bundle();
        let bundle = Bundle::load(&other).unwrap();
        fs::remove_file(other.join("app/bin/tool")).unwrap();
        assert!(bundle.verify_payload().is_err());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn rejects_bundles_requiring_a_newer_client_and_other_runtimes() {
        let root = generic_bundle();
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(root.join("bundle.json")).unwrap()).unwrap();
        manifest["metadata"]["minToolVersion"] = json!("99.0.0");
        write_manifest(&root, &manifest);
        let bundle = Bundle::load(&root).unwrap();
        let error = bundle.ensure_compatible().unwrap_err();
        assert!(error.contains("升级"), "unexpected: {error}");

        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(root.join("bundle.json")).unwrap()).unwrap();
        manifest["metadata"]["minToolVersion"] = json!("0.0.1");
        manifest["runtime"]["type"] = json!("kubernetes");
        write_manifest(&root, &manifest);
        let error = Bundle::load(&root).unwrap_err();
        assert!(error.contains("暂不支持"), "unexpected: {error}");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deployed_record_lists_declared_artifacts() {
        let root = generic_bundle();
        let bundle = Bundle::load(&root).unwrap();
        let record = bundle.deployed_record("p1");
        assert_eq!(record.project_id, "p1");
        assert_eq!(record.version, "12-ab12cd3");
        assert_eq!(record.artifacts.len(), 3);
        assert_eq!(record.artifacts[0].root, "deploy");
        fs::remove_dir_all(root).unwrap();
    }
}
