// Deployment adapters and their internal bundle, transport, and execution boundaries.
mod models;
pub use models::{
    RemoteArtifactStatus, RemoteDeploymentStatus, RemoteUpdateProgress, RemoteUpdateResult,
};

mod bundle;

mod plan;
pub use plan::{
    __cmd__inspect_bundle_plan, inspect_bundle_plan, inspect_bundle_plan_blocking, BundlePlan,
    PlanFile, PlanImage, PlanStep,
};

mod config;
pub use config::{
    __cmd__inspect_bundle_config, inspect_bundle_config, BundleConfigView, ComposeImageView,
    ComposeServiceView,
};
pub(super) use config::{
    compose_service_views, declared_images, match_tarball, normalize_image_ref,
    resolve_bundle_file, scan_image_files,
};

mod inspect;
pub use inspect::{
    __cmd__get_remote_containers_status, __cmd__get_remote_service_logs,
    __cmd__inspect_remote_deployment, get_remote_containers_status, get_remote_service_logs,
    inspect_remote_deployment,
};

mod scripts;
mod transfer;
pub(crate) use transfer::upload_file_with_progress;

mod update;
pub use update::{__cmd__update_remote_deployment, update_remote_deployment};

mod install;
pub use install::{__cmd__install_bundle, install_bundle, InstallBundleResult};

mod progress;

#[cfg(test)]
mod tests;
