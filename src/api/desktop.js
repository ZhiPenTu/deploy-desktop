import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';

export function isDesktopApp() {
  return isTauri();
}

function requireDesktop() {
  if (!isDesktopApp()) throw new Error('当前操作暂不可用，请在桌面应用中重试。');
}

export async function invokeDesktop(command, args) {
  requireDesktop();
  return await invoke(command, args);
}

export async function invokeWithProgress(command, args, onProgress) {
  requireDesktop();
  const channel = new Channel();
  if (typeof onProgress === 'function') channel.onmessage = onProgress;
  return await invoke(command, { ...args, onEvent: channel });
}

export function listenProgress(event, callback) {
  if (!isDesktopApp()) return () => {};
  return listen(event, (message) => callback(message.payload));
}

export async function selectLocalPath(options) {
  requireDesktop();
  return await open(options);
}
