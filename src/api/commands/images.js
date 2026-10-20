import { invokeDesktop, listenProgress } from '../desktop.js';

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
  return listenProgress("image-library-progress", callback);
}

export function onImageManagerProgress(callback) {
  return listenProgress("image-manager-progress", callback);
}
