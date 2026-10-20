import { invokeDesktop } from '../desktop.js';

export async function checkLocalEnv(targetDir = null) {
  return await invokeDesktop("check_local_env", { targetDir });
}

export async function checkRemoteEnv(host, port, user, keyPath = null) {
  return await invokeDesktop("check_remote_env", { host, port, user, keyPath });
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

export async function remoteDockerComposeAction(host, port, user, keyPath, deployDir, action = "up", service = null) {
  return await invokeDesktop("remote_docker_compose_action", { host, port, user, keyPath, deployDir, action, service });
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
