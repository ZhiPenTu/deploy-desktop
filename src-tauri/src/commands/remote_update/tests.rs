use super::config::compose_image_status;
use super::*;
use crate::commands::bundle::sha256_hex;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn temp_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "deploy-desktop-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// 按目录实际内容生成 files[]，测试只声明产物与组件。
fn files_of(root: &Path) -> Vec<Value> {
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(dir) = fs::read_dir(&current) else { continue };
        for entry in dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
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
        left["path"].as_str().unwrap_or_default().cmp(right["path"].as_str().unwrap_or_default())
    });
    entries
}

fn write_bundle(root: &Path, compose: &str, artifacts: Value, components: Value, images: Value) {
    let manifest = json!({
        "apiVersion": "deploy.desktop/bundle/v2",
        "kind": "DeploymentBundle",
        "metadata": {
            "product": "demo",
            "displayName": "演示",
            "version": "12-ab12cd3",
            "commit": "ab12cd3",
            "commitShort": "ab12cd3",
            "branch": "main",
            "builtAt": "2026-10-21T08:00:00Z"
        },
        "runtime": {
            "type": "compose",
            "compose": compose,
            "containerPrefix": "demo",
            "services": [
                { "key": "web", "label": "网页", "port": 8080, "protocol": "TCP" },
                { "key": "api", "label": "接口", "port": 9000, "protocol": "TCP" }
            ]
        },
        "components": components,
        "artifacts": artifacts,
        "images": images,
        "plan": { "steps": [
            { "key": "verify", "title": "验收页面", "hint": "打开首页确认版本号" },
            { "key": "custom", "title": "通知值班", "hint": "部署后告知值班同事" }
        ] },
        "files": files_of(root),
    });
    write(
        &root.join("bundle.json"),
        &serde_json::to_string_pretty(&manifest).unwrap(),
    );
}

fn generic_bundle(root: &Path) -> std::path::PathBuf {
    let bundle = root.join("bundle");
    write(&bundle.join("app/web/index.html"), "<h1>hi</h1>");
    write(&bundle.join("stack/docker-compose.yml"), "services:\n  web:\n    image: demo/web:12@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n");
    write_bundle(
        &bundle,
        "stack/docker-compose.yml",
        json!([
            { "key": "web", "label": "网页产物", "source": "app/web", "root": "deploy",
              "path": "site", "type": "dir" },
            { "key": "compose", "label": "部署配置", "source": "stack/docker-compose.yml",
              "root": "deploy", "path": "docker-compose.yml", "type": "file" }
        ]),
        json!([
            { "key": "site", "label": "网站", "artifacts": ["web"], "services": ["web"] },
            { "key": "config", "label": "部署配置", "artifacts": ["compose"], "services": [] }
        ]),
        json!([
            { "name": "web", "ref": "demo/web:12", "role": "app", "required": true,
              "bundleFile": "images/web.tar" }
        ]),
    );
    write(&bundle.join("images/web.tar"), "image archive");
    // bundle.json 生成后补入镜像包，需重新计算文件清单。
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(bundle.join("bundle.json")).unwrap()).unwrap();
    let mut manifest = manifest;
    manifest["files"] = json!(files_of(&bundle));
    write(
        &bundle.join("bundle.json"),
        &serde_json::to_string_pretty(&manifest).unwrap(),
    );
    bundle
}

#[test]
fn install_distributes_only_declared_artifacts() {
    let root = temp_root("v2-install");
    let bundle_dir = generic_bundle(&root);
    let bundle = crate::commands::bundle::Bundle::load(&bundle_dir).unwrap();
    bundle.verify_payload().unwrap();

    let components = super::bundle::install_components(&bundle);
    assert_eq!(components, vec!["site", "config", "image"]);
    let selected = bundle.select_components(&components).unwrap();
    let install = root.join("install");
    let deploy = install.join(bundle.deploy_source_rel());
    let resolved = bundle
        .resolve_artifacts(&selected, &install, &deploy)
        .unwrap();
    assert_eq!(resolved.len(), 2);

    let mut written = Vec::new();
    for artifact in &resolved {
        match artifact.kind {
            crate::commands::bundle::ArtifactKind::File => {
                fs::create_dir_all(artifact.target.parent().unwrap()).unwrap();
                fs::copy(&artifact.source, &artifact.target).unwrap();
            }
            crate::commands::bundle::ArtifactKind::Dir => {
                fs::create_dir_all(&artifact.target).unwrap();
                fs::copy(
                    artifact.source.join("index.html"),
                    artifact.target.join("index.html"),
                )
                .unwrap();
            }
        }
        written.push(artifact.target.to_string_lossy().to_string());
    }
    assert!(install.join("stack/site/index.html").is_file());
    assert!(install.join("stack/docker-compose.yml").is_file());
    assert_eq!(written.len(), 2);

    let record = bundle.deployed_record("p1");
    crate::commands::bundle::write_deployed_record(&install, &record).unwrap();
    let reloaded = crate::commands::bundle::read_deployed_record(&install).unwrap();
    assert_eq!(reloaded.version, "12-ab12cd3");
    assert_eq!(reloaded.artifacts.len(), 2);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn plan_uses_descriptor_components_and_declared_steps() {
    let root = temp_root("v2-plan");
    let bundle_dir = generic_bundle(&root);
    let plan = inspect_bundle_plan_blocking(bundle_dir.to_string_lossy().to_string(), None, None, None, None)
        .unwrap();
    assert_eq!(plan.product, "demo");
    assert_eq!(plan.compose, "stack/docker-compose.yml");
    assert_eq!(plan.components.len(), 2);
    assert_eq!(plan.components[0].key, "site");
    assert_eq!(plan.components[0].artifacts[0].source, "app/web");
    assert_eq!(plan.components[0].artifacts[0].target, "site");
    assert_eq!(plan.components[0].artifacts[0].root, "deploy");
    assert_eq!(plan.services.len(), 2);
    assert_eq!(plan.file_count, 3);
    assert_eq!(plan.images.len(), 1);
    assert_eq!(plan.images[0].role, "app");
    assert!(plan.images[0].required);

    let verify = plan.steps.iter().find(|step| step.key == "verify").unwrap();
    assert_eq!(verify.title, "验收页面");
    assert_eq!(verify.hint, "打开首页确认版本号");
    let custom = plan.steps.iter().find(|step| step.key == "custom").unwrap();
    assert_eq!(custom.title, "通知值班");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn plan_rejects_bundles_without_compose_artifact() {
    let root = temp_root("v2-plan-invalid");
    let bundle = root.join("bundle");
    write(&bundle.join("stack/docker-compose.yml"), "services: {}\n");
    write_bundle(
        &bundle,
        "stack/docker-compose.yml",
        json!([]),
        json!([]),
        json!([]),
    );
    let error = inspect_bundle_plan_blocking(bundle.to_string_lossy().to_string(), None, None, None, None)
        .unwrap_err();
    assert!(
        error.contains("Compose") || error.contains("未归属于"),
        "unexpected: {error}"
    );
    fs::remove_dir_all(root).unwrap();
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

    let (hit, source) = match_tarball("demo-frontend:dev", &files, &[], &[]);
    assert_eq!(source, "filename");
    assert_eq!(hit.map(|(name, _)| name).unwrap_or_default(), "frontend.tar");

    let (hit, source) = match_tarball("bellsoft/liberica-openjdk-rocky:21-cds", &files, &[], &[]);
    assert_eq!(source, "filename");
    assert_eq!(hit.map(|(name, _)| name).unwrap_or_default(), "openjdk.tar");
}

#[test]
fn declared_images_resolve_bundle_files() {
    let root = temp_root("v2-declared-images");
    let bundle_dir = generic_bundle(&root);
    let declared = declared_images(&bundle_dir);
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].file, "web.tar");
    assert_eq!(declared[0].reference, "demo/web:12");

    let files = vec![("web.tar".to_string(), 13)];
    let (hit, source) = match_tarball("demo/web:12", &files, &[], &declared);
    assert_eq!(source, "bundle");
    assert_eq!(hit.map(|(name, _)| name).unwrap_or_default(), "web.tar");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn compose_image_does_not_report_missing_when_docker_is_unavailable() {
    assert_eq!(
        compose_image_status(false, false, "unavailable"),
        "unconfirmed"
    );
    assert_eq!(compose_image_status(false, false, "local"), "missing");
    assert_eq!(compose_image_status(true, false, "unavailable"), "to_import");
    assert_eq!(compose_image_status(false, true, "remote"), "loaded");
    assert_eq!(compose_image_status(true, true, "local"), "ready");
}

#[test]
fn bundle_config_rejects_paths_outside_bundle() {
    let root = temp_root("compose-guard");
    let bundle = root.join("bundle");
    write(&bundle.join("deploy/docker-compose.yml"), "name: demo");

    let error = super::config::inspect_bundle_config_blocking(
        bundle.to_string_lossy().to_string(),
        "../secret.txt".to_string(),
        None,
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

#[test]
fn compose_package_directory_preserves_explicit_selection_and_bundled_images() {
    use super::config::config_image_dir;

    let root = temp_root("compose-directory");
    let bundle = root.join("bundle");
    let library = root.join("offline-library");
    let selected = root.join("selected-media");
    write(&bundle.join("deploy/docker-compose.yml"), "services: {}\n");
    write(&library.join("cache.tar"), "library archive");
    write(&selected.join("images/cache.tar"), "selected archive");

    assert_eq!(config_image_dir(&bundle, None, Some(&library)), library);
    assert_eq!(config_image_dir(&bundle, Some("  "), Some(&library)), library);

    write(&bundle.join("cache.tar"), "root archive");
    assert_eq!(config_image_dir(&bundle, None, Some(&library)), bundle);
    write(&bundle.join("images/cache.tar"), "bundled archive");
    assert_eq!(config_image_dir(&bundle, None, Some(&library)), bundle.join("images"));
    assert_eq!(
        config_image_dir(&bundle, selected.to_str(), Some(&library)),
        selected.join("images")
    );

    let missing = root.join("missing-media");
    assert_eq!(config_image_dir(&bundle, missing.to_str(), Some(&library)), missing);

    fs::remove_dir_all(&root).ok();
}
