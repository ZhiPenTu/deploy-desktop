import { invokeRead, invokeDesktop } from '../desktop.js';

export async function inspectDeploymentBackups(installDir) {
  return await invokeRead('inspect_deployment_backups', { installDir });
}

export async function cleanDeploymentBackups(installDir) {
  return await invokeDesktop('clean_deployment_backups', { installDir });
}

export async function inspectRemoteDeploymentBackups(host, port, user, keyPath, installDir) {
  return await invokeRead('inspect_remote_deployment_backups', {
    host,
    port,
    user,
    keyPath,
    installDir
  });
}

export async function cleanRemoteDeploymentBackups(host, port, user, keyPath, installDir) {
  return await invokeDesktop('clean_remote_deployment_backups', {
    host,
    port,
    user,
    keyPath,
    installDir
  });
}
