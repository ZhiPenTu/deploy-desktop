import { invokeDesktop, invokeWithProgress, isDesktopApp } from '../desktop.js';

export async function listArtifactSources() {
  if (!isDesktopApp()) return [];
  return await invokeDesktop('list_artifact_sources');
}

export async function saveArtifactSource(source) {
  return await invokeDesktop('save_artifact_source', { source });
}

export async function deleteArtifactSource(id) {
  return await invokeDesktop('delete_artifact_source', { id });
}

export async function listArtifactTokens() {
  if (!isDesktopApp()) return [];
  return await invokeDesktop('list_artifact_tokens');
}

export async function saveArtifactToken(token) {
  return await invokeDesktop('save_artifact_token', { token });
}

export async function deleteArtifactToken(id) {
  return await invokeDesktop('delete_artifact_token', { id });
}

export async function fetchArtifactRelease(id, runId = null) {
  return await invokeDesktop('fetch_artifact_release', { id, runId });
}

export async function listArtifactReleases(id, page = 1) {
  return await invokeDesktop('list_artifact_releases', { id, page });
}

export async function listCachedArtifactReleases(id) {
  return await invokeDesktop('list_cached_artifact_releases', { id });
}

export async function downloadArtifact(id, release, onProgress) {
  return await invokeWithProgress('download_artifact', { id, release }, onProgress);
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
  if (!isDesktopApp()) {
    window.open(url, '_blank', 'noopener');
    return;
  }
  return await invokeDesktop('open_external_url', { url });
}

export async function prepareLocalArtifactBundle(path) {
  return await invokeDesktop('prepare_local_artifact_bundle', { path });
}
