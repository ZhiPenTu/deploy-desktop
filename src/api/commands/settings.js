import { invokeDesktop, invokeWithProgress } from '../desktop.js';

export async function checkAppUpdate() {
  return await invokeDesktop('check_app_update', {});
}

export async function installAppUpdate(onProgress) {
  return await invokeWithProgress('install_app_update', {}, onProgress);
}
