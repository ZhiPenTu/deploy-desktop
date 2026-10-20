import { Channel, invoke, isTauri } from '@tauri-apps/api/core';

const NO_TAURI = '浏览器预览模式不支持云端制品操作，请在桌面端运行。';

export async function listArtifactSources() {
  if (!isTauri()) return [];
  return await invoke('list_artifact_sources');
}

export async function saveArtifactSource(source) {
  if (!isTauri()) throw new Error(NO_TAURI);
  return await invoke('save_artifact_source', { source });
}

export async function deleteArtifactSource(id) {
  if (!isTauri()) throw new Error(NO_TAURI);
  return await invoke('delete_artifact_source', { id });
}

export async function fetchArtifactRelease(id, runId = null) {
  if (!isTauri()) throw new Error(NO_TAURI);
  return await invoke('fetch_artifact_release', { id, runId });
}

export async function listArtifactReleases(id, page = 1) {
  if (!isTauri()) throw new Error(NO_TAURI);
  return await invoke('list_artifact_releases', { id, page });
}

export async function downloadArtifact(id, release, onProgress) {
  if (!isTauri()) throw new Error(NO_TAURI);
  const channel = new Channel();
  if (typeof onProgress === 'function') {
    channel.onmessage = onProgress;
  }
  return await invoke('download_artifact', { id, release, onEvent: channel });
}

export async function openArtifactBundle(id) {
  if (!isTauri()) throw new Error(NO_TAURI);
  return await invoke('open_artifact_bundle', { id });
}

export async function openArtifactBundleVersion(id, version) {
  if (!isTauri()) throw new Error(NO_TAURI);
  return await invoke('open_artifact_bundle_version', { id, version });
}

export async function deleteLocalBundle(id) {
  if (!isTauri()) throw new Error(NO_TAURI);
  return await invoke('delete_local_bundle', { id });
}
