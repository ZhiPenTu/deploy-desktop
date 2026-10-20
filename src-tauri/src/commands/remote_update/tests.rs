use super::bundle::{install_bundle_components, validate_bundle};
use super::config::compose_image_status;
use super::install::install_bundle_local;
use super::models::BundlePaths;
use std::fs;
use std::path::Path;

use super::*;

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

#[test]
fn first_install_validates_image_bundle_without_requiring_app_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-image-install-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bundle = root.join("bundle");
    write(&bundle.join("images/web.tar"), "image archive");
    write(
        &bundle.join("deploy/docker-compose.yml"),
        "services:\n  web:\n    image: demo/web:1\n",
    );
    let components = install_bundle_components(bundle.to_str().unwrap()).unwrap();
    assert_eq!(components, ["image", "config"]);
    let paths = validate_bundle(bundle.to_str().unwrap(), &components).unwrap();
    assert!(paths.server_jar.is_none());
    assert_eq!(paths.image_files.len(), 1);
    let install = root.join("install");
    install_bundle_local(&paths, &bundle, install.to_str().unwrap()).unwrap();
    assert!(install.join("images/web.tar").is_file());
    assert!(install.join("deploy/docker-compose.yml").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn first_install_keeps_complete_file_bundle_validation() {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-file-install-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    write(&root.join("app/frontend-dist/index.html"), "frontend");
    write(&root.join("deploy/docker-compose.yml"), "services: {}");
    let components = install_bundle_components(root.to_str().unwrap()).unwrap();
    assert_eq!(components, ["backend", "gateway", "frontend", "config"]);
    assert!(validate_bundle(root.to_str().unwrap(), &components).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_bundle_with_base_images_still_distributes_application_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-file-images-install-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bundle = root.join("bundle");
    let artifact = crate::commands::profile::deployment_profile()
        .backend_artifact
        .clone();
    write(&bundle.join("app").join(&artifact), "backend artifact");
    write(
        &bundle.join("app/gateway-classes/Main.class"),
        "gateway classes",
    );
    write(&bundle.join("app/frontend-dist/index.html"), "frontend");
    write(&bundle.join("images/base.tar"), "base image archive");
    write(&bundle.join("deploy/docker-compose.yml"), "services: {}\n");
    // Legacy packages without a mode prefer application artifacts over auxiliary images.
    assert_eq!(
        inspect_bundle_layout(bundle.to_str().unwrap().to_string())
            .unwrap()
            .mode,
        "file"
    );
    write(
        &bundle.join("bundle.json"),
        r#"{"mode":"file","version":"1"}"#,
    );
    let components = install_bundle_components(bundle.to_str().unwrap()).unwrap();
    assert_eq!(components, ["backend", "gateway", "frontend", "config"]);
    let paths = validate_bundle(bundle.to_str().unwrap(), &components).unwrap();
    let install = root.join("install");
    install_bundle_local(&paths, &bundle, install.to_str().unwrap()).unwrap();
    assert!(install
        .join("backend-java/server/target")
        .join(&artifact)
        .is_file());
    assert!(install
        .join("backend-java/gateway-app/target/classes/Main.class")
        .is_file());
    assert!(install.join("deploy/frontend_dist/index.html").is_file());
    assert!(install.join("images/base.tar").is_file());
    // An explicit valid package declaration remains authoritative.
    write(&bundle.join("bundle.json"), r#"{"mode":"image"}"#);
    assert_eq!(
        inspect_bundle_layout(bundle.to_str().unwrap().to_string())
            .unwrap()
            .mode,
        "image"
    );
    write(&bundle.join("bundle.json"), r#"{"mode":"other"}"#);
    assert!(inspect_bundle_layout(bundle.to_str().unwrap().to_string()).is_err());
    write(&bundle.join("bundle.json"), r#"{"mode":1}"#);
    assert!(inspect_bundle_layout(bundle.to_str().unwrap().to_string()).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn declared_file_components_are_validated_before_install_without_requiring_gateway() {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-components-install-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let artifact = crate::commands::profile::deployment_profile()
        .backend_artifact
        .clone();
    write(&root.join("app").join(artifact), "backend artifact");
    write(&root.join("app/frontend-dist/index.html"), "frontend");
    write(&root.join("deploy/docker-compose.yml"), "services: {}\n");
    write(
        &root.join("bundle.json"),
        r#"{"mode":"file","components":["backend","frontend","image","frontend","unknown"]}"#,
    );
    assert_eq!(
        install_bundle_components(root.to_str().unwrap()).unwrap(),
        ["backend", "frontend", "config"]
    );
    assert!(validate_install_bundle(root.to_str().unwrap()).is_ok());
    write(&root.join("bundle.json"), r#"{"mode":"file"}"#);
    assert!(validate_install_bundle(root.to_str().unwrap()).is_err());
    write(
        &root.join("bundle.json"),
        r#"{"mode":"file","components":"backend"}"#,
    );
    assert!(validate_install_bundle(root.to_str().unwrap()).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_install_places_bundle_into_compose_mount_layout() {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-install-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bundle = root.join("bundle");
    let install = root.join("install");
    write(&bundle.join("app/server.jar"), "jar");
    write(
        &bundle.join("app/frontend-dist/index.html"),
        "<html></html>",
    );
    write(&bundle.join("app/gateway-classes/Application.class"), "x");
    write(&bundle.join("app/gateway-lib/dependency.jar"), "x");
    write(&bundle.join("deploy/docker-compose.yml"), "name: demo");
    write(&bundle.join("deploy/docker-compose.lan.yml"), "name: lan");
    write(&bundle.join("deploy/nginx/udp-lb.conf"), "events {}");
    write(
        &bundle.join("deploy/nginx/frontend-default.conf"),
        "server {}",
    );
    write(
        &bundle.join("deploy/nginx/frontend-upstream.conf"),
        "upstream backend {}",
    );
    write(&bundle.join("deploy/nginx/frontend-main.conf"), "http {}");

    let paths = BundlePaths {
        server_jar: Some(bundle.join("app/server.jar")),
        gateway_classes: Some(bundle.join("app/gateway-classes")),
        gateway_lib: Some(bundle.join("app/gateway-lib")),
        frontend_dist: Some(bundle.join("app/frontend-dist")),
        compose_file: Some(bundle.join("deploy/docker-compose.yml")),
        compose_lan_file: Some(bundle.join("deploy/docker-compose.lan.yml")),
        nginx_dir: Some(bundle.join("deploy/nginx")),
        oceanbase_dir: None,
        image_files: Vec::new(),
    };

    install_bundle_local(&paths, &bundle, install.to_str().unwrap()).unwrap();

    let artifact = &crate::commands::profile::deployment_profile().backend_artifact;
    assert!(install
        .join("backend-java/server/target")
        .join(artifact)
        .is_file());
    assert!(install
        .join("backend-java/gateway-app/target/classes/Application.class")
        .is_file());
    assert!(install
        .join("backend-java/gateway-app/target/lib/dependency.jar")
        .is_file());
    assert!(install.join("deploy/frontend_dist/index.html").is_file());
    assert!(install.join("deploy/docker-compose.yml").is_file());
    assert!(install.join("deploy/docker-compose.lan.yml").is_file());
    assert!(install.join("deploy/nginx/udp-lb.conf").is_file());
    assert!(install.join("deploy/nginx/frontend-default.conf").is_file());
    assert!(install
        .join("deploy/nginx/frontend-upstream.conf")
        .is_file());
    assert!(install.join("deploy/nginx/frontend-main.conf").is_file());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn plan_marks_file_bundle_base_images_optional() {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-plan-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bundle = root.join("bundle");
    write(&bundle.join("app/tt-net-manager-server.jar"), "jar");
    write(
        &bundle.join("app/frontend-dist/index.html"),
        "<html></html>",
    );
    write(&bundle.join("deploy/docker-compose.yml"), "name: demo");
    write(
        &bundle.join("deploy/base-images.txt"),
        "# 注释\nredis:7.2.8\nnginx:1.27-alpine\n",
    );
    write(
        &bundle.join("bundle.json"),
        "{\"version\":\"42\",\"mode\":\"file\"}",
    );

    let plan =
        inspect_bundle_plan_blocking(bundle.to_string_lossy().to_string(), None, None, None, None)
            .unwrap();

    assert_eq!(plan.mode, "file");
    assert_eq!(plan.version, "42");
    assert!(plan
        .app_files
        .iter()
        .any(|file| file.path == "app/tt-net-manager-server.jar"));
    assert!(plan
        .config_files
        .iter()
        .any(|file| file.path == "deploy/docker-compose.yml"));
    assert_eq!(plan.images.len(), 2);
    assert!(plan.images.iter().all(|image| image.source == "declared"));
    let image_step = plan.steps.iter().find(|step| step.key == "images").unwrap();
    // 文件模式的基础镜像不是交付包内容：本机齐备时为 ready，缺镜像时为 optional，
    // 但绝不会因为「缺镜像」而阻塞（镜像模式才会 blocked）。
    assert_ne!(image_step.state, "blocked");
    assert!(image_step.title.contains("基础镜像"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn local_install_copies_image_bundle_files() {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-install-images-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bundle = root.join("bundle");
    let install = root.join("install");
    write(&bundle.join("images/app-stack-42.tar.gz"), "tar");
    write(&bundle.join("images/images.json"), "{}");
    write(&bundle.join("deploy/docker-compose.yml"), "name: demo");
    write(&bundle.join("deploy/docker-compose.lan.yml"), "name: lan");

    let paths = BundlePaths {
        server_jar: None,
        gateway_classes: None,
        gateway_lib: None,
        frontend_dist: None,
        compose_file: Some(bundle.join("deploy/docker-compose.yml")),
        compose_lan_file: Some(bundle.join("deploy/docker-compose.lan.yml")),
        nginx_dir: None,
        oceanbase_dir: None,
        image_files: vec![bundle.join("images/app-stack-42.tar.gz")],
    };

    install_bundle_local(&paths, &bundle, install.to_str().unwrap()).unwrap();

    assert!(install.join("images/app-stack-42.tar.gz").is_file());
    assert!(install.join("images/images.json").is_file());
    assert!(install.join("deploy/docker-compose.lan.yml").is_file());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn compose_config_lists_services_and_images() {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-compose-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bundle = root.join("bundle");
    write(
        &bundle.join("deploy/docker-compose.yml"),
        "name: demo\nservices:\n  kafka:\n    image: bitnamilegacy/kafka:4.0\n    container_name: demo-kafka\n    ports:\n      - \"9092:9092\"\n    depends_on:\n      - redis\n  backend:\n    image: bellsoft/liberica-openjdk-rocky:21-cds\n    depends_on:\n      redis:\n        condition: service_started\n  redis:\n    image: redis:7.2.8\n",
    );
    write(&bundle.join("images/redis.tar"), "tar");
    write(
        &bundle.join("images/images.json"),
        "{\"files\":[{\"file\":\"redis.tar\",\"images\":[\"redis:7.2.8\"]}]}",
    );

    let view = inspect_bundle_config(
        bundle.to_string_lossy().to_string(),
        "deploy/docker-compose.yml".to_string(),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(view.kind, "compose");
    assert_eq!(view.services.len(), 3);
    assert_eq!(view.images.len(), 3);
    assert!(view.content.contains("services:"));

    let kafka = view
        .services
        .iter()
        .find(|service| service.name == "kafka")
        .unwrap();
    assert_eq!(kafka.container_name, "demo-kafka");
    assert_eq!(kafka.ports, vec!["9092:9092".to_string()]);
    assert_eq!(kafka.depends_on, vec!["redis".to_string()]);

    let backend = view
        .services
        .iter()
        .find(|service| service.name == "backend")
        .unwrap();
    assert_eq!(backend.depends_on, vec!["redis".to_string()]);

    let redis = view
        .images
        .iter()
        .find(|image| image.match_ref == "redis:7.2.8")
        .unwrap();
    assert_eq!(redis.local_file, "redis.tar");
    assert_eq!(redis.match_source, "images.json");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn compose_digest_and_filename_fallback_match_local_tarballs() {
    assert_eq!(
        normalize_image_ref(
            "oceanbase/oceanbase-ce:4.4.2-lts@sha256:eaa2c3b3513fa21e4a658f197c1deb93c9d6b6eb742cfa5ef0f085f82b9cec54"
        ),
        "oceanbase/oceanbase-ce:4.4.2-lts"
    );
    assert_eq!(
        normalize_image_ref("docker.io/library/nginx:1.27-alpine"),
        "nginx:1.27-alpine"
    );

    let files = vec![
        ("frontend.tar".to_string(), 10),
        ("openjdk.tar".to_string(), 20),
    ];

    let (hit, source) = match_tarball("tt-net-manager-frontend:dev", &files, &[], &[]);
    assert_eq!(source, "filename");
    assert_eq!(
        hit.map(|(name, _)| name).unwrap_or_default(),
        "frontend.tar"
    );

    let (hit, source) = match_tarball("bellsoft/liberica-openjdk-rocky:21-cds", &files, &[], &[]);
    assert_eq!(source, "filename");
    assert_eq!(hit.map(|(name, _)| name).unwrap_or_default(), "openjdk.tar");
}

#[test]
fn compose_image_does_not_report_missing_when_docker_is_unavailable() {
    assert_eq!(
        compose_image_status(false, false, "unavailable"),
        "unconfirmed"
    );
    assert_eq!(compose_image_status(false, false, "local"), "missing");
    assert_eq!(
        compose_image_status(true, false, "unavailable"),
        "to_import"
    );
    assert_eq!(compose_image_status(false, true, "remote"), "loaded");
    assert_eq!(compose_image_status(true, true, "local"), "ready");
}

#[test]
fn bundle_config_rejects_paths_outside_bundle() {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-compose-guard-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bundle = root.join("bundle");
    write(&bundle.join("deploy/docker-compose.yml"), "name: demo");

    let error = inspect_bundle_config(
        bundle.to_string_lossy().to_string(),
        "../secret.txt".to_string(),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap_err();
    assert!(error.contains("不合法"), "unexpected error: {error}");

    fs::remove_dir_all(&root).ok();
}
