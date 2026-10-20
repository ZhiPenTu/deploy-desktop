import { invokeDesktop, listenProgress, isDesktopApp } from '../desktop.js';

export async function listRegistrySources() {
  if (!isDesktopApp()) return [];
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
  return listenProgress("registry-progress", callback);
}
