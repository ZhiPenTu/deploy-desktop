import { invokeDesktop, invokeRead, invokeWithProgress } from '../desktop.js';

export async function scanUsbMedia(mediaDir) {
  return await invokeRead('scan_usb_media', { mediaDir });
}

export async function listLocalArtifactBundles() {
  return await invokeRead('list_local_artifact_bundles');
}

export async function verifyUsbEntry(mediaDir, entryId) {
  return await invokeDesktop('verify_usb_entry', { mediaDir, entryId });
}

export async function exportUsbMedia(mediaDir, artifactDirs, imageFiles, onProgress) {
  return await invokeWithProgress(
    'export_usb_media',
    { mediaDir, artifactDirs, imageFiles },
    onProgress
  );
}

export async function importUsbImage(mediaDir, entryId, onProgress) {
  return await invokeWithProgress('import_usb_image', { mediaDir, entryId }, onProgress);
}
