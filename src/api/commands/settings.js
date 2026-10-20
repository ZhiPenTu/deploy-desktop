import { invokeDesktop, invokeWithProgress, isDesktopApp } from '../desktop.js';

export async function getDeploymentProfile() {
  if (!isDesktopApp()) return null;
  return await invokeDesktop('get_deployment_profile');
}

export async function checkAppUpdate() {
  return await invokeDesktop('check_app_update', {});
}

export async function installAppUpdate(onProgress) {
  return await invokeWithProgress('install_app_update', {}, onProgress);
}
