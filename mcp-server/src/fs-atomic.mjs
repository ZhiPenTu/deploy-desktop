import fs from 'node:fs/promises';
import path from 'node:path';

let sequence = 0;

async function syncDirectory(dir) {
  if (process.platform === 'win32') return;
  let handle;
  try {
    handle = await fs.open(dir, 'r');
    await handle.sync();
  } catch {
    // 部分文件系统不支持目录 fsync；写入本身已完成，这里不作为失败处理。
  } finally {
    await handle?.close().catch(() => {});
  }
}

/**
 * 覆盖写入本机配置文件。
 *
 * 先写唯一临时文件并落盘，再整体改名替换目标，读者只会看到旧内容或新内容，
 * 不会读到写了一半的 JSON。调用方负责加锁；这里不感知业务锁，避免嵌套加锁死锁。
 */
export async function writePrivateFile(target, content) {
  const dir = path.dirname(target);
  await fs.mkdir(dir, { recursive: true, mode: 0o700 });
  if (process.platform !== 'win32') await fs.chmod(dir, 0o700).catch(() => {});

  sequence = (sequence + 1) % Number.MAX_SAFE_INTEGER;
  const temp = `${target}.${process.pid}.${Date.now()}.${sequence}.tmp`;
  const handle = await fs.open(temp, 'wx', 0o600);
  try {
    await handle.writeFile(content);
    await handle.sync();
  } catch (error) {
    await handle.close().catch(() => {});
    await fs.rm(temp, { force: true }).catch(() => {});
    throw error;
  }
  await handle.close();
  try {
    if (process.platform !== 'win32') await fs.chmod(temp, 0o600).catch(() => {});
    await fs.rename(temp, target);
  } catch (error) {
    await fs.rm(temp, { force: true }).catch(() => {});
    throw error;
  }
  if (process.platform !== 'win32') await fs.chmod(target, 0o600).catch(() => {});
  await syncDirectory(dir);
}

/** 私有配置文件内容格式化：缩进与结尾换行在两端保持一致，便于人工排查。 */
export function formatJson(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

/** 读取 JSON；文件不存在返回 null，内容为空或损坏时抛出解析错误交给调用方翻译。 */
export async function readJsonFile(file) {
  let text;
  try {
    text = await fs.readFile(file, 'utf8');
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw error;
  }
  return JSON.parse(text);
}
