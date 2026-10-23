import { invokeDesktop, invokeRead, invokeWithProgress } from '../desktop.js';

export async function checkDockerInstallation() {
  return await invokeRead('check_docker_installation');
}

export async function openDockerInstallation() {
  return await invokeDesktop('open_docker_installation');
}

export async function checkAppUpdate() {
  return await invokeDesktop('check_app_update', {});
}

export async function installAppUpdate(onProgress) {
  return await invokeWithProgress('install_app_update', {}, onProgress);
}
