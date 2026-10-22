import { invokeDesktop } from '../desktop.js';

export async function readRuntimeLogs(level = '', limit = 500, keyword = '') {
  return await invokeDesktop('read_runtime_logs', { level, limit, keyword });
}

export async function clearRuntimeLogs() {
  return await invokeDesktop('clear_runtime_logs');
}

export async function openRuntimeLogDir() {
  return await invokeDesktop('open_runtime_log_dir');
}

export async function getRuntimeLogSettings() {
  return await invokeDesktop('get_runtime_log_settings');
}

export async function setRuntimeLogDir(dir) {
  return await invokeDesktop('set_runtime_log_dir', { dir: dir || null });
}

export async function setRuntimeLogLevel(level) {
  return await invokeDesktop('set_runtime_log_level', { level });
}

export async function reportClientEvent(level, target, message) {
  return await invokeDesktop('log_client_event', { level, target, message });
}
