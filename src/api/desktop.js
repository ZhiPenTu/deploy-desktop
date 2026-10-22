import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';

export function isDesktopApp() {
  return isTauri();
}

function requireDesktop() {
  if (!isDesktopApp()) throw new Error('当前操作暂不可用，请在桌面应用中重试。');
}

// 最近的失败只记一次，避免同一个错误在多个页面重复刷屏。
const recentFailures = new Map();

function reportCommandFailure(command, error) {
  if (!isDesktopApp()) return;
  const message = String(error?.message || error || '未知错误').slice(0, 2000);
  const key = `${command}|${message}`;
  const now = Date.now();
  for (const [known, loggedAt] of recentFailures) {
    if (now - loggedAt > 5000) recentFailures.delete(known);
  }
  if (recentFailures.has(key)) return;
  recentFailures.set(key, now);
  invoke('log_client_event', { level: 'error', target: command, message }).catch(() => {});
}

// 详细调试开关由运行日志页面控制；开启后每次命令调用都会留下一条调试记录。
let commandTraceEnabled = false;

export function setCommandTrace(enabled) {
  commandTraceEnabled = Boolean(enabled);
}

function traceCommand(command, startedAt, detail) {
  if (!commandTraceEnabled || !isDesktopApp()) return;
  const elapsed = Math.max(0, Math.round(performance.now() - startedAt));
  const message = detail ? `耗时 ${elapsed}ms - ${String(detail).slice(0, 500)}` : `耗时 ${elapsed}ms`;
  invoke('log_client_event', { level: 'debug', target: command, message }).catch(() => {});
}

export async function reportClientEvent(level, target, message) {
  if (!isDesktopApp()) return;
  return await invoke('log_client_event', {
    level,
    target,
    message: String(message || '').slice(0, 2000)
  });
}

export async function invokeDesktop(command, args) {
  requireDesktop();
  const startedAt = performance.now();
  try {
    const result = await invoke(command, args);
    traceCommand(command, startedAt);
    return result;
  } catch (error) {
    traceCommand(command, startedAt, `失败：${error?.message || error}`);
    reportCommandFailure(command, error);
    throw error;
  }
}

const pendingReads = new Map();

// Share only pending reads across page mounts. Results and failures are never cached.
export async function invokeRead(command, args) {
  requireDesktop();
  const key = JSON.stringify([command, args]);
  if (!pendingReads.has(key)) {
    const startedAt = performance.now();
    const pending = invoke(command, args)
      .then((result) => {
        traceCommand(command, startedAt);
        return result;
      })
      .catch((error) => {
        traceCommand(command, startedAt, `失败：${error?.message || error}`);
        reportCommandFailure(command, error);
        throw error;
      })
      .finally(() => pendingReads.delete(key));
    pendingReads.set(key, pending);
  }
  return await pendingReads.get(key);
}

export async function invokeWithProgress(command, args, onProgress) {
  requireDesktop();
  const channel = new Channel();
  if (typeof onProgress === 'function') channel.onmessage = onProgress;
  const startedAt = performance.now();
  try {
    const result = await invoke(command, { ...args, onEvent: channel });
    traceCommand(command, startedAt);
    return result;
  } catch (error) {
    traceCommand(command, startedAt, `失败：${error?.message || error}`);
    reportCommandFailure(command, error);
    throw error;
  }
}

export function listenProgress(event, callback) {
  if (!isDesktopApp()) return () => {};
  return listen(event, (message) => callback(message.payload));
}

export async function selectLocalPath(options) {
  requireDesktop();
  return await open(options);
}
