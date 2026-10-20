import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

function requireDesktop() {
  if (!isTauri()) throw new Error('当前操作暂不可用，请在桌面应用中重试。');
}

async function invokeDesktop(command, args) {
  requireDesktop();
  return await invoke(command, args);
}

export async function getDeploymentProfile() {
  if (!isTauri()) return null;
  return await invokeDesktop('get_deployment_profile');
}

export async function checkAppUpdate() {
  return await invokeDesktop('check_app_update', {});
}

export async function installAppUpdate(onProgress) {
  requireDesktop();
  const channel = new Channel();
  if (typeof onProgress === 'function') channel.onmessage = onProgress;
  return await invokeDesktop('install_app_update', { onEvent: channel });
}

export async function listArtifactSources() {
  if (!isTauri()) return [];
  return await invokeDesktop('list_artifact_sources');
}

export async function saveArtifactSource(source) {
  return await invokeDesktop('save_artifact_source', { source });
}

export async function deleteArtifactSource(id) {
  return await invokeDesktop('delete_artifact_source', { id });
}

export async function fetchArtifactRelease(id, runId = null) {
  return await invokeDesktop('fetch_artifact_release', { id, runId });
}

export async function listArtifactReleases(id, page = 1) {
  return await invokeDesktop('list_artifact_releases', { id, page });
}

export async function downloadArtifact(id, release, onProgress) {
  requireDesktop();
  const channel = new Channel();
  if (typeof onProgress === 'function') channel.onmessage = onProgress;
  return await invokeDesktop('download_artifact', { id, release, onEvent: channel });
}

export async function openArtifactBundle(id) {
  return await invokeDesktop('open_artifact_bundle', { id });
}

export async function openArtifactBundleVersion(id, version) {
  return await invokeDesktop('open_artifact_bundle_version', { id, version });
}

export async function deleteLocalBundle(id) {
  return await invokeDesktop('delete_local_bundle', { id });
}

export async function openArtifactRunUrl(url) {
  if (!isTauri()) {
    window.open(url, '_blank', 'noopener');
    return;
  }
  return await invokeDesktop('open_external_url', { url });
}

export async function checkLocalEnv(targetDir = null) {
  return await invokeDesktop("check_local_env", { targetDir });
}

export async function installOfflineDockerEngine(bundleDir) {
  return await invokeDesktop("install_offline_docker_engine", { bundleDir });
}

export async function loadOfflineImages(bundleDir) {
  return await invokeDesktop("load_offline_images", { bundleDir });
}

export async function scanLocalImageBundle(dir) {
  return await invokeDesktop("scan_local_image_bundle", { dir });
}

export async function inspectRemoteImageBundle(host, port, user, keyPath, bundleDir) {
  return await invokeDesktop("inspect_remote_image_bundle", { host, port, user, keyPath, bundleDir });
}

export async function uploadRemoteImageBundle(host, port, user, keyPath, localDir, remoteBundleDir, files, overwrite = false) {
  return await invokeDesktop("upload_remote_image_bundle", {
    host,
    port,
    user,
    keyPath,
    localDir,
    remoteBundleDir,
    files,
    overwrite
  });
}

export async function importRemoteImageBundle(host, port, user, keyPath, remoteBundleDir, files) {
  return await invokeDesktop("import_remote_image_bundle", {
    host,
    port,
    user,
    keyPath,
    remoteBundleDir,
    files
  });
}

export async function removeRemoteImageFiles(host, port, user, keyPath, remoteBundleDir, files) {
  return await invokeDesktop("remove_remote_image_files", {
    host,
    port,
    user,
    keyPath,
    remoteBundleDir,
    files
  });
}

export async function getImageLibrarySettings() {
  return await invokeDesktop("get_image_library_settings");
}

export async function saveImageLibrarySettings(offlineDir) {
  return await invokeDesktop("save_image_library_settings", { offlineDir });
}

export async function listLocalDockerImages() {
  return await invokeDesktop("list_local_docker_images");
}

export async function pullLocalDockerImage(reference, username = null, password = null, registrySourceId = null) {
  return await invokeDesktop("pull_local_docker_image", { reference, username, password, registrySourceId });
}

export async function listRegistrySources() {
  if (!isTauri()) return [];
  return await invokeDesktop('list_registry_sources');
}

export async function saveRegistrySource(source) {
  return await invokeDesktop('save_registry_source', { source });
}

export async function deleteRegistrySource(id) {
  return await invokeDesktop('delete_registry_source', { id });
}

export async function testRegistrySource(id) {
  return await invokeDesktop('test_registry_source', { id });
}

export async function removeLocalDockerImages(references) {
  return await invokeDesktop("remove_local_docker_images", { references });
}

export async function exportLocalDockerImages(references) {
  return await invokeDesktop("export_local_docker_images", { references });
}

export async function importOfflinePackages(files) {
  return await invokeDesktop("import_offline_packages", { files });
}

export async function scanOfflinePackages() {
  return await invokeDesktop("scan_offline_packages");
}

export async function removeOfflinePackages(files) {
  return await invokeDesktop("remove_offline_packages", { files });
}

export function onImageLibraryProgress(callback) {
  if (!isTauri()) return () => {};
  return listen("image-library-progress", (event) => callback(event.payload));
}

export async function getContainersStatus() {
  return await invokeDesktop("get_containers_status");
}

export async function dockerComposeAction(deployDir, action, service = null) {
  return await invokeDesktop("docker_compose_action", { deployDir, action, service });
}

export async function getServiceLogs(containerName, tail = 200) {
  return await invokeDesktop("get_service_logs", { containerName, tail });
}

export async function checkRemoteEnv(host, port, user, keyPath = null) {
  return await invokeDesktop("check_remote_env", { host, port, user, keyPath });
}

export async function installRemoteDockerEngine(host, port, user, keyPath, bundleDir) {
  return await invokeDesktop("install_remote_docker_engine", { host, port, user, keyPath, bundleDir });
}

export async function remoteDockerComposeAction(host, port, user, keyPath, deployDir, action = "up", service = null) {
  return await invokeDesktop("remote_docker_compose_action", { host, port, user, keyPath, deployDir, action, service });
}

export async function inspectBundlePlan(bundleDir, target = null) {
  return await invokeDesktop('inspect_bundle_plan', {
    bundleDir,
    host: target?.host || null,
    port: target?.port ?? null,
    user: target?.user || null,
    keyPath: target?.keyPath || null
  });
}

export async function inspectBundleConfig(bundleDir, file, target = null, imageDir = '') {
  return await invokeDesktop('inspect_bundle_config', {
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
  return await invokeDesktop('inspect_deployment_review', {
    bundleDir,
    installDir,
    imageDir: imageDir || null,
    host: target?.host || null,
    port: target?.port ?? null,
    user: target?.user || null,
    keyPath: target?.keyPath || null
  });
}

export async function inspectRemoteDeployment(host, port, user, keyPath, installDir) {
  return await invokeDesktop("inspect_remote_deployment", { host, port, user, keyPath, installDir });
}

export async function getRemoteContainersStatus(host, port, user, keyPath, installDir) {
  return await invokeDesktop("get_remote_containers_status", { host, port, user, keyPath, installDir });
}

export async function getRemoteServiceLogs(host, port, user, keyPath, containerName, tail = 200) {
  return await invokeDesktop("get_remote_service_logs", { host, port, user, keyPath, containerName, tail });
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
  if (!isTauri()) return () => {};
  return listen("remote-update-progress", (event) => callback(event.payload));
}

export function onImageManagerProgress(callback) {
  if (!isTauri()) return () => {};
  return listen("image-manager-progress", (event) => callback(event.payload));
}

export async function loadImageManifest(source, registrySourceId = null) {
  return await invokeDesktop("load_image_manifest", { source, registrySourceId });
}

export async function pullManifestImagesRemote(
  host,
  port,
  user,
  keyPath,
  source,
  username,
  password,
  imageNames,
  exportDir = null,
  registrySourceId = null
) {
  return await invokeDesktop("pull_manifest_images_remote", {
    host,
    port,
    user,
    keyPath,
    source,
    username,
    password,
    imageNames,
    exportDir,
    registrySourceId
  });
}

export async function pullManifestImagesLocal(
  source,
  username,
  password,
  imageNames,
  exportDir = null,
  registrySourceId = null
) {
  return await invokeDesktop("pull_manifest_images_local", {
    source,
    username,
    password,
    imageNames,
    exportDir,
    registrySourceId
  });
}

export function onRegistryProgress(callback) {
  if (!isTauri()) return () => {};
  return listen("registry-progress", (event) => callback(event.payload));
}

export async function listConnectionInventory() {
  if (!isTauri()) {
    return { credentials: [], hosts: [], activeHostId: '' };
  }
  return await invokeDesktop('list_connection_inventory');
}

export async function saveSshCredential(credential) {
  return await invokeDesktop('save_ssh_credential', { credential });
}

export async function deleteSshCredential(id) {
  return await invokeDesktop('delete_ssh_credential', { id });
}

export async function saveManagedHost(host) {
  return await invokeDesktop('save_managed_host', { host });
}

export async function deleteManagedHost(id) {
  return await invokeDesktop('delete_managed_host', { id });
}

export async function setActiveManagedHost(id) {
  return await invokeDesktop('set_active_managed_host', { id });
}

export async function testManagedHostConnection(id) {
  return await invokeDesktop('test_managed_host_connection', { id });
}

export async function setupManagedHostAccess(id) {
  return await invokeDesktop('setup_managed_host_access', { id });
}

export async function prepareLocalArtifactBundle(path) {
  return await invokeDesktop('prepare_local_artifact_bundle', { path });
}

export async function updateLocalDeployment(bundleDir, installDir, components, backup = true, dryRun = false) {
  return await invokeDesktop('update_local_deployment', { bundleDir, installDir, components, backup, dryRun });
}

export function isDesktopApp() {
  return isTauri();
}
