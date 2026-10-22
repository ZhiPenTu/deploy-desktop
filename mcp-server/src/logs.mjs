import fs from 'node:fs/promises';
import path from 'node:path';
import { DeployError } from './errors.mjs';
import { appConfigDir, defaultLogDir } from './paths.mjs';
import { expandHome } from './util.mjs';

export { appConfigDir, defaultLogDir };

export const LOG_FILE = 'runtime.log';
export const BACKUP_FILE = 'runtime.1.log';
export const MAX_TOTAL_BYTES = 10 * 1024 * 1024;

const LEVEL_PATTERN = /^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}(?:\.\d+)?) (INFO|WARN|ERROR|DEBUG|TRACE) (.*)$/;

export const SETTINGS_FILE = 'log-settings.json';

export async function configuredLogDir() {
  try {
    const text = await fs.readFile(path.join(appConfigDir(), SETTINGS_FILE), 'utf8');
    const parsed = JSON.parse(text);
    const dir = typeof parsed?.dir === 'string' ? parsed.dir.trim() : '';
    if (dir && path.isAbsolute(dir)) return expandHome(dir);
  } catch {
    return null;
  }
  return null;
}

/** 应用里设置过日志目录时优先使用，其次才是平台默认目录。 */
export async function runtimeLogDir() {
  const override = process.env.DEPLOY_DESKTOP_LOG_DIR;
  if (override) return expandHome(override);
  return (await configuredLogDir()) || defaultLogDir();
}

export function parseLogLine(line) {
  const trimmed = String(line || '').trimEnd();
  const match = LEVEL_PATTERN.exec(trimmed);
  if (!match) {
    return {
      time: '',
      date: '',
      level: 'INFO',
      target: 'app',
      message: trimmed
    };
  }
  const [, date, clock, level, rest] = match;
  const separator = rest.indexOf(' - ');
  const target = separator === -1 ? 'app' : rest.slice(0, separator).trim();
  const message = separator === -1 ? rest.trim() : rest.slice(separator + 3).trim();
  return { time: `${date} ${clock}`, date, level, target, message };
}

function matchesLevel(entryLevel, filter) {
  switch (String(filter || '').trim().toLowerCase()) {
    case 'info':
      return entryLevel === 'INFO';
    case 'warn':
    case 'warning':
      return entryLevel === 'WARN';
    case 'error':
      return entryLevel === 'ERROR';
    case 'debug':
      return entryLevel === 'DEBUG' || entryLevel === 'TRACE';
    default:
      return true;
  }
}

function matchesKeyword(entry, keyword) {
  if (!keyword) return true;
  const needle = keyword.toLowerCase();
  return entry.message.toLowerCase().includes(needle) || entry.target.toLowerCase().includes(needle);
}

export async function logFiles() {
  const dir = await runtimeLogDir();
  const files = [];
  for (const name of [BACKUP_FILE, LOG_FILE]) {
    const file = path.join(dir, name);
    try {
      const stat = await fs.stat(file);
      if (stat.isFile()) files.push({ path: file, name, size: stat.size });
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
  }
  return { dir, files };
}

export async function readAllEntries() {
  const { dir, files } = await logFiles();
  if (files.length === 0) {
    throw new DeployError('尚未找到本机运行日志，请先在部署舱中执行一次下载或部署操作', {
      code: 'LOG_NOT_FOUND',
      hint: `预期日志目录：${dir}`,
      details: { dir }
    });
  }
  const entries = [];
  for (const file of files) {
    const content = await fs.readFile(file.path, 'utf8');
    for (const line of content.split('\n')) {
      if (!line.trim()) continue;
      entries.push({ ...parseLogLine(line), file: file.name });
    }
  }
  return { dir, files, entries };
}

export function countEntries(entries) {
  const counts = { info: 0, warn: 0, error: 0, debug: 0 };
  for (const entry of entries) {
    if (entry.level === 'ERROR') counts.error += 1;
    else if (entry.level === 'WARN') counts.warn += 1;
    else if (entry.level === 'DEBUG' || entry.level === 'TRACE') counts.debug += 1;
    else counts.info += 1;
  }
  return counts;
}

export async function readRuntimeLogs({ level = '', limit = 100, keyword = '' } = {}) {
  const { dir, files, entries } = await readAllEntries();
  const needle = String(keyword || '').trim();
  const matched = entries.filter(
    (entry) => matchesLevel(entry.level, level) && matchesKeyword(entry, needle)
  );
  const total = Math.max(1, Math.min(1000, Number(limit) || 100));
  const selected = matched.slice(-total).reverse();
  return {
    dir,
    level: String(level || 'all'),
    keyword: needle,
    counts: countEntries(entries),
    matched: matched.length,
    returned: selected.length,
    truncated: matched.length > selected.length,
    totalBytes: files.reduce((sum, file) => sum + file.size, 0),
    maxBytes: MAX_TOTAL_BYTES,
    entries: selected.map(({ time, level: entryLevel, target, message }) => ({
      time,
      level: entryLevel,
      target,
      message
    }))
  };
}

export async function runtimeLogSummary() {
  const { dir, files, entries } = await readAllEntries();
  const lastOf = (level) => {
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      if (entries[index].level === level) return entries[index];
    }
    return null;
  };
  const first = entries[0];
  const last = entries[entries.length - 1];
  return {
    dir,
    files: files.map((file) => ({ name: file.name, bytes: file.size })),
    totalBytes: files.reduce((sum, file) => sum + file.size, 0),
    maxBytes: MAX_TOTAL_BYTES,
    totalCount: entries.length,
    counts: countEntries(entries),
    firstTime: first ? first.time : '',
    lastTime: last ? last.time : '',
    latestWarning: lastOf('WARN'),
    latestError: lastOf('ERROR')
  };
}
