pub mod commands;

use tauri::Manager;

use commands::{
    artifact::*,
    connections::*,
    deployment_review::*,
    db_ops::*,
    docker_engine::*,
    docker_ops::*,
    env::*,
    image_library::*,
    image_manager::*,
    local_update::*,
    profile::*,
    registry::*,
    registry_sources::*,
    remote_ssh::*,
    remote_update::*,
    updater::*,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            commands::profile::init_profile(app.handle());
            app.manage(commands::updater::PendingUpdate::default());
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            clear_stale_webview_cache(app);
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            check_local_env,
            install_offline_docker_engine,
            load_offline_images,
            get_containers_status,
            docker_compose_action,
            get_service_logs,
            check_oceanbase_status,
            import_oceanbase_sql,
            test_ssh_connection,
            setup_remote_ssh_key,
            check_remote_env,
            install_remote_docker_engine,
            load_remote_images,
            remote_docker_compose_action,
            check_remote_oceanbase_status,
            import_remote_oceanbase_sql,
            run_remote_deploy_action,
            inspect_remote_deployment,
            inspect_deployment_review,
            update_local_deployment,
            inspect_bundle_layout,
            inspect_bundle_plan,
            inspect_bundle_config,
            install_bundle,
            get_remote_containers_status,
            get_remote_service_logs,
            update_remote_deployment,
            scan_local_image_bundle,
            get_image_library_settings,
            save_image_library_settings,
            list_local_docker_images,
            pull_local_docker_image,
            remove_local_docker_images,
            export_local_docker_images,
            import_offline_packages,
            scan_offline_packages,
            remove_offline_packages,
            inspect_remote_image_bundle,
            upload_remote_image_bundle,
            import_remote_image_bundle,
            remove_remote_image_files,
            load_image_manifest,
            list_registry_sources,
            save_registry_source,
            delete_registry_source,
            test_registry_source,
            pull_manifest_images_remote,
            pull_manifest_images_local,
            get_deployment_profile,
            check_app_update,
            install_app_update,
            get_update_settings,
            save_update_settings,
            list_artifact_sources,
            save_artifact_source,
            delete_artifact_source,
            list_artifact_releases,
            list_connection_inventory,
            save_ssh_credential,
            delete_ssh_credential,
            save_managed_host,
            delete_managed_host,
            set_active_managed_host,
            test_managed_host_connection,
            setup_managed_host_access,
            fetch_artifact_release,
            download_artifact,
            open_artifact_bundle,
            prepare_local_artifact_bundle,
            open_artifact_bundle_version,
            delete_local_bundle,
            open_external_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn clear_stale_webview_cache(app: &tauri::App) {
    use tauri::Manager;

    let version = app.package_info().version.to_string();
    let Ok(config_dir) = app.path().app_config_dir() else {
        return;
    };
    let marker = config_dir.join("webview-cache-version");
    let previous = std::fs::read_to_string(&marker)
        .ok()
        .map(|value| value.trim().to_string());
    if previous.as_deref() == Some(version.as_str()) {
        return;
    }

    let Ok(home) = app.path().home_dir() else {
        return;
    };
    let identifier = app.config().identifier.as_str();
    let _ = std::fs::remove_dir_all(home.join("Library/WebKit").join(identifier));
    let _ = std::fs::remove_dir_all(home.join("Library/Caches").join(identifier));
    let _ = std::fs::create_dir_all(&config_dir);
    let _ = std::fs::write(marker, version);
}
