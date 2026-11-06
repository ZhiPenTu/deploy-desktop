import { invokeRead, invokeDesktop, invokeWithProgress, listenProgress } from '../desktop.js';

export async function installOfflineDockerEngine(bundleDir) {
  return await invokeDesktop("install_offline_docker_engine", { bundleDir });
}

export async function installRemoteDockerEngine(host, port, user, keyPath, bundleDir) {
  return await invokeDesktop("install_remote_docker_engine", { host, port, user, keyPath, bundleDir });
}

export async function listDockerOfflinePackages() {
  return await invokeRead("list_docker_offline_packages");
}

export async function downloadDockerOfflinePackage(item = {}, savePath, onProgress) {
  return await invokeWithProgress(
    "download_docker_offline_package",
    {
      platform: item.platform,
      arch: item.arch,
      version: item.version,
      build: item.build || "",
      savePath
    },
    onProgress
  );
}

export async function loadOfflineImages(bundleDir) {
  return await invokeDesktop("load_offline_images", { bundleDir });
}

export async function exportDeploymentImages(bundleDir, references, target) {
  return await invokeDesktop('export_deployment_images', {
    bundleDir,
    references,
    host: target?.host || '',
    port: target?.port ?? 22,
    user: target?.user || '',
    keyPath: target?.keyPath || null
  });
}

export async function pullDeploymentImages(bundleDir, references, target = null, registrySourceId = null, password = null) {
  return await invokeDesktop('pull_deployment_images', {
    bundleDir,
    references,
    host: target?.host || null,
    port: target?.port ?? null,
    user: target?.user || null,
    keyPath: target?.keyPath || null,
    registrySourceId: registrySourceId || null,
    password: password || null
  });
}

export async function inspectBundlePlan(bundleDir, target = null) {
  return await invokeRead('inspect_bundle_plan', {
    bundleDir,
    host: target?.host || null,
    port: target?.port ?? null,
    user: target?.user || null,
    keyPath: target?.keyPath || null
  });
}

export async function inspectBundleConfig(bundleDir, file, target = null, imageDir = '') {
  return await invokeRead('inspect_bundle_config', {
    bundleDir,
    file,
    imageDir: imageDir || null,
    host: target?.host || null,
    port: target?.port ?? null,
    user: target?.user || null,
    keyPath: target?.keyPath || null
  });
}

export async function inspectDeploymentReview(bundleDir, installDir, target = null, imageDir = '') {
  return await invokeRead('inspect_deployment_review', {
    bundleDir,
    installDir,
    imageDir: imageDir || null,
    host: target?.host || null,
    port: target?.port ?? null,
    user: target?.user || null,
    keyPath: target?.keyPath || null
  });
}

export async function updateRemoteDeployment(host, port, user, keyPath, bundleDir, installDir, components, backup, dryRun = false) {
  return await invokeDesktop("update_remote_deployment", {
    host,
    port,
    user,
    keyPath,
    bundleDir,
    installDir,
    components,
    backup,
    dryRun
  });
}

export async function updateLocalDeployment(bundleDir, installDir, components, backup = true, dryRun = false) {
  return await invokeDesktop('update_local_deployment', { bundleDir, installDir, components, backup, dryRun });
}

export async function installBundle(mode, host, port, user, keyPath, bundleDir, installDir) {
  return await invokeDesktop("install_bundle", {
    mode,
    host,
    port,
    user,
    keyPath,
    bundleDir,
    installDir
  });
}

export function onRemoteUpdateProgress(callback) {
  return listenProgress("remote-update-progress", callback);
}
