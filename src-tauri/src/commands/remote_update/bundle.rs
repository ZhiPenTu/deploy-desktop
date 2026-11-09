use crate::commands::bundle::{default_components, normalize_components, Bundle};
use crate::commands::projects;
use std::path::Path;

pub(super) fn normalize_install_dir(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        projects::active_project()
            .map(|project| project.install_dir)
            .unwrap_or_default()
    } else {
        trimmed.to_string()
    }
}

/// 读取并校验目标交付包，版本不满足要求时直接报错。
pub(super) fn validate_install_dir(value: &str) -> Result<(), String> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() || value.contains(['\n', '\r', '\0']) || !value.starts_with('/') {
        return Err(String::from(
            "部署目录必须是目标机上的绝对路径，且不能包含上级目录",
        ));
    }
    let mut has_normal = false;
    for part in Path::new(value).components() {
        match part {
            std::path::Component::Normal(_) => has_normal = true,
            std::path::Component::RootDir => {}
            _ => {
                return Err(String::from(
                    "部署目录必须是目标机上的绝对路径，且不能包含上级目录",
                ))
            }
        }
    }
    if !has_normal {
        return Err(String::from(
            "部署目录不能是目标机的系统根目录，请填写专用部署路径",
        ));
    }
    Ok(())
}

pub(super) fn load_target(path: &Path) -> Result<Bundle, String> {
    let bundle = Bundle::load(path)?;
    bundle.ensure_compatible()?;
    Ok(bundle)
}

/// 读取并校验交付包，返回包与本次要更新的组件键。
pub(super) fn load_bundle(
    bundle_dir: &str,
    components: &[String],
) -> Result<(Bundle, Vec<String>), String> {
    let bundle = Bundle::load(Path::new(bundle_dir.trim()))?;
    bundle.ensure_compatible()?;
    let components = normalize_components(&bundle, components.to_vec())?;
    Ok((bundle, components))
}

/// 首次部署需要分发的全部内容，按交付包声明的组件顺序排列。
pub(super) fn install_components(bundle: &Bundle) -> Vec<String> {
    default_components(bundle)
}
