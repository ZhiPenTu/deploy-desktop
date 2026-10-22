import fs from 'node:fs/promises';
import { DeployError } from './errors.mjs';

const RETRY_MS = 15;
const TIMEOUT_MS = 3000;
const STALE_MS = 30_000;

/**
 * 与桌面端共用的配置锁文件名。桌面端在 Rust 侧使用同一后缀与同一套约定，
 * 两端抢同一把锁，读改写才不会互相覆盖。
 */
export function lockPath(target) {
  return `${target}.lock`;
}

const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function tryAcquire(lock) {
  try {
    const handle = await fs.open(lock, 'wx', 0o600);
    try {
      await handle.writeFile(`${process.pid}\n`);
    } finally {
      await handle.close();
    }
    return true;
  } catch (error) {
    if (error.code === 'EEXIST') return false;
    throw error;
  }
}

async function isStale(lock) {
  try {
    const stat = await fs.stat(lock);
    return Date.now() - stat.mtimeMs > STALE_MS;
  } catch {
    return false;
  }
}

/**
 * 在跨进程锁内执行一次读改写。桌面端正同时修改同一份配置时会短暂等待；
 * 超时给出可重试的提示，而不是让两个进程各自写一半。
 */
export async function withFileLock(target, action, options = {}) {
  const lock = lockPath(target);
  const timeoutMs = options.timeoutMs ?? TIMEOUT_MS;
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (await tryAcquire(lock)) break;
    if (await isStale(lock)) {
      await fs.rm(lock, { force: true }).catch(() => {});
      continue;
    }
    if (Date.now() >= deadline) {
      throw new DeployError('配置正在被另一个程序写入，请稍后重试', { code: 'CONFIG_LOCKED' });
    }
    await delay(RETRY_MS);
  }
  try {
    return await action();
  } finally {
    await fs.rm(lock, { force: true }).catch(() => {});
  }
}
