import os from 'node:os';
import path from 'node:path';

export function expandHome(input) {
  if (typeof input !== 'string') return input;
  const value = input.trim();
  if (value === '~') return os.homedir();
  if (value.startsWith('~/') || value.startsWith('~\\')) {
    return path.join(os.homedir(), value.slice(2));
  }
  return value;
}

export function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`;
}

export function nowStamp(date = new Date()) {
  const pad = (n) => String(n).padStart(2, '0');
  return [
    date.getFullYear(),
    pad(date.getMonth() + 1),
    pad(date.getDate()),
    '-',
    pad(date.getHours()),
    pad(date.getMinutes()),
    pad(date.getSeconds())
  ].join('');
}

export function formatBytes(bytes) {
  const value = Number(bytes) || 0;
  if (value < 1024) return `${value} B`;
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MB`;
  return `${(value / 1024 ** 3).toFixed(2)} GB`;
}

export function parseBoolean(value) {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    return ['1', 'true', 'yes', 'on'].includes(value.toLowerCase());
  }
  return false;
}

// 与桌面端 core/version.rs 共用 fixtures/versions.json，两侧规则必须完全一致。
function versionSegments(value) {
  const text = String(value ?? '').trim();
  const body = text.replace(/^[vV]/, '');
  if (!body) return [0];
  return body.split(/[.+-]/).map((part) => {
    const segment = part.trim();
    return /^\d+$/.test(segment) ? Number.parseInt(segment, 10) : 0;
  });
}

export function compareVersions(left, right) {
  const a = versionSegments(left);
  const b = versionSegments(right);
  const length = Math.max(a.length, b.length);
  for (let index = 0; index < length; index += 1) {
    const x = a[index] ?? 0;
    const y = b[index] ?? 0;
    if (x !== y) return x < y ? -1 : 1;
  }
  return 0;
}

export function clamp(value, min, max) {
  const number = Number(value);
  if (!Number.isFinite(number)) return min;
  return Math.min(max, Math.max(min, number));
}

export function unique(values) {
  return [...new Set(values)];
}

export function isObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export function pick(object, keys) {
  const result = {};
  for (const key of keys) {
    if (object[key] !== undefined) result[key] = object[key];
  }
  return result;
}

export function sanitizeFileName(name) {
  const value = String(name || '').trim();
  if (!value || value === '.' || value === '..' || value.includes('/') || value.includes('\\')) {
    throw new Error(`非法的文件名: ${name}`);
  }
  return value;
}

export function isArchiveFile(name) {
  return /\.(tar|tgz)$/i.test(name) || /\.tar\.gz$/i.test(name);
}
