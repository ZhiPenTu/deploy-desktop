// 直接读取 gzip/tar 流解包，不依赖系统 tar。
//
// Windows 自带 tar 会把无编码声明的 UTF-8 文件名按本地代码页解码，包含中文
// 文件名的交付包会在解包中途失败，且失败原因只留在子进程 stderr 里。这里
// 逐条校验并写入，仍拒绝绝对路径、父目录跳转、链接与特殊文件。

import { createReadStream } from 'node:fs';
import fs from 'node:fs/promises';
import path from 'node:path';
import { compose } from 'node:stream';
import zlib from 'node:zlib';
import { DeployError } from './errors.mjs';

const BLOCK_SIZE = 512;
const MAX_EXTENSION_BYTES = 1024 * 1024;
const NAME_DECODER = new TextDecoder('utf-8', { fatal: true });

const RESERVED_NAMES = new Set([
  'CON',
  'PRN',
  'AUX',
  'NUL',
  'CONIN$',
  'CONOUT$',
  ...Array.from({ length: 10 }, (_, index) => `COM${index}`),
  ...Array.from({ length: 10 }, (_, index) => `LPT${index}`),
  'COM¹',
  'COM²',
  'COM³',
  'LPT¹',
  'LPT²',
  'LPT³'
]);

function archiveError(message, code, details) {
  return new DeployError(message, { code, details });
}

function invalidArchive(reason) {
  return archiveError(
    '构建物不是有效的 tar.gz 压缩包，请重新下载后重试',
    'ARTIFACT_ARCHIVE_INVALID',
    reason ? { reason } : undefined
  );
}

function unsafeArchive(message, details) {
  return archiveError(message, 'ARTIFACT_ARCHIVE_UNSAFE', details);
}

function extractFailed(file, reason) {
  return archiveError('解包交付包失败，请确认磁盘空间充足后重试', 'ARTIFACT_EXTRACT_FAILED', {
    file,
    reason: reason?.message || String(reason)
  });
}

class TruncatedArchive extends Error {}

class StreamReader {
  constructor(stream) {
    this.stream = stream;
    this.iterator = stream[Symbol.asyncIterator]();
    this.chunks = [];
    this.length = 0;
    this.ended = false;
  }

  async pull() {
    if (this.ended) return false;
    const { value, done } = await this.iterator.next();
    if (done) {
      this.ended = true;
      return false;
    }
    const chunk = Buffer.isBuffer(value) ? value : Buffer.from(value);
    this.chunks.push(chunk);
    this.length += chunk.length;
    return true;
  }

  consume(size) {
    if (size === 0) return Buffer.alloc(0);
    const joined = this.chunks.length === 1 ? this.chunks[0] : Buffer.concat(this.chunks, this.length);
    const head = joined.subarray(0, size);
    const rest = joined.subarray(size);
    this.chunks = rest.length > 0 ? [rest] : [];
    this.length = rest.length;
    return head;
  }

  async readExact(size) {
    while (this.length < size) {
      if (!(await this.pull())) throw new TruncatedArchive('压缩包内容不完整');
    }
    return this.consume(size);
  }

  async copy(size, write) {
    let remaining = size;
    while (remaining > 0) {
      if (this.length === 0 && !(await this.pull())) {
        throw new TruncatedArchive('压缩包内容不完整');
      }
      const take = Math.min(remaining, this.length);
      await write(this.consume(take));
      remaining -= take;
    }
  }

  async skip(size) {
    await this.copy(size, async () => {});
  }

  destroy() {
    this.stream.destroy();
  }
}

function parseOctal(buffer) {
  if (buffer.length > 0 && (buffer[0] & 0x80) !== 0) return null;
  const text = buffer.toString('latin1').replace(/[\0 ]+$/g, '').trim();
  if (text === '') return 0;
  if (!/^[0-7]+$/.test(text)) return null;
  return Number.parseInt(text, 8);
}

function parseNumber(buffer) {
  if (buffer.length === 0 || (buffer[0] & 0x80) === 0) return parseOctal(buffer);
  if ((buffer[0] & 0x40) !== 0) return null;
  let value = 0n;
  for (let index = 0; index < buffer.length; index += 1) {
    const byte = index === 0 ? buffer[index] & 0x7f : buffer[index];
    value = (value << 8n) | BigInt(byte);
  }
  if (value > BigInt(Number.MAX_SAFE_INTEGER)) return null;
  return Number(value);
}

function decodeUtf8(buffer) {
  const end = buffer.indexOf(0);
  const value = end === -1 ? buffer : buffer.subarray(0, end);
  return NAME_DECODER.decode(value);
}

function verifyChecksum(header) {
  const stored = parseOctal(header.subarray(148, 156));
  if (stored === null) throw invalidArchive('校验和字段无效');
  let unsigned = 0;
  let signed = 0;
  for (let index = 0; index < BLOCK_SIZE; index += 1) {
    const byte = index >= 148 && index < 156 ? 0x20 : header[index];
    unsigned += byte;
    signed += byte > 0x7f ? byte - 0x100 : byte;
  }
  if (stored !== unsigned && stored !== signed) throw invalidArchive('校验和不一致');
}

export function isSafeEntry(name) {
  if (!name || name.includes('\0')) return false;
  if (name.startsWith('/') || name.includes('\\')) return false;
  if (/^[A-Za-z]:/.test(name)) return false;
  return !name
    .split('/')
    .some((part) => part === '..' || !isPortableComponent(part));
}

function isPortableComponent(part) {
  if (!part || part === '.') return true;
  if (part.endsWith('.') || part.endsWith(' ') || part.includes(':')) return false;
  if (/[<>"|?*\x00-\x1f]/.test(part)) return false;
  const stem = part.split('.')[0].replace(/ +$/, '').toUpperCase();
  return !RESERVED_NAMES.has(stem);
}

function permissionMode(mode, fallback) {
  const value = (Number(mode) || 0) & 0o777;
  return value === 0 ? fallback : value & ~0o022;
}

function paddingFor(size) {
  return (BLOCK_SIZE - (size % BLOCK_SIZE)) % BLOCK_SIZE;
}

function parsePax(buffer) {
  const records = {};
  let offset = 0;
  while (offset < buffer.length) {
    const space = buffer.indexOf(0x20, offset);
    if (space === -1) break;
    const lengthText = buffer.subarray(offset, space).toString('latin1');
    if (!/^[0-9]+$/.test(lengthText)) throw invalidArchive('扩展头格式无效');
    const length = Number.parseInt(lengthText, 10);
    if (!Number.isInteger(length) || length <= 0 || offset + length > buffer.length) {
      throw invalidArchive('扩展头格式无效');
    }
    let record;
    try {
      record = NAME_DECODER.decode(buffer.subarray(space + 1, offset + length));
    } catch {
      throw invalidArchive('扩展头编码无效');
    }
    const line = record.endsWith('\n') ? record.slice(0, -1) : record;
    const equals = line.indexOf('=');
    if (equals > 0) records[line.slice(0, equals)] = line.slice(equals + 1);
    offset += length;
  }
  if (Object.keys(records).some((key) => key.startsWith('GNU.sparse.'))) {
    throw unsafeArchive('交付包包含链接或特殊文件，无法解压，请重新打包后重试');
  }
  return records;
}

async function readExtension(reader, size) {
  if (size > MAX_EXTENSION_BYTES) throw invalidArchive('扩展头过大');
  return reader.readExact(size);
}

async function consumeEndMarker(reader) {
  const second = await reader.readExact(BLOCK_SIZE);
  if (!second.every((byte) => byte === 0)) throw invalidArchive('归档结束标记不完整');
  for (;;) {
    if (reader.length === 0 && !(await reader.pull())) break;
    const rest = reader.consume(reader.length);
    if (rest.some((byte) => byte !== 0)) throw invalidArchive('归档结束标记后存在多余数据');
  }
}

async function writeAll(handle, chunk) {
  let offset = 0;
  while (offset < chunk.length) {
    const { bytesWritten } = await handle.write(chunk, offset, chunk.length - offset);
    if (!bytesWritten) throw new Error('写入文件失败');
    offset += bytesWritten;
  }
}

async function writeFileEntry(reader, target, name, size, mode) {
  let handle;
  try {
    handle = await fs.open(target, 'w', 0o666);
    await reader.copy(size, (chunk) => writeAll(handle, chunk));
  } catch (error) {
    if (error instanceof TruncatedArchive) throw invalidArchive(error.message);
    throw extractFailed(name, error);
  } finally {
    await handle?.close().catch(() => {});
  }
  if (process.platform !== 'win32') {
    try {
      await fs.chmod(target, permissionMode(mode, 0o644));
    } catch (error) {
      throw extractFailed(name, error);
    }
  }
}

export async function safeExtract(archive, destination, options = {}) {
  const maxBytes = options.maxBytes ?? Number.MAX_SAFE_INTEGER;
  const maxFiles = options.maxFiles ?? Number.MAX_SAFE_INTEGER;
  try {
    await fs.mkdir(destination, { recursive: true, mode: 0o700 });
  } catch (error) {
    throw extractFailed(destination, error);
  }

  const reader = new StreamReader(compose(createReadStream(archive), zlib.createGunzip()));
  const directories = [];
  let pendingName = null;
  let pendingPax = null;
  let files = 0;
  let bytes = 0;
  let entries = 0;
  let sawEndMarker = false;

  try {
    for (;;) {
      if (reader.length === 0 && !(await reader.pull())) break;
      if (reader.length < BLOCK_SIZE) throw invalidArchive('压缩包尾部不完整');
      const header = reader.consume(BLOCK_SIZE);
      if (header.every((byte) => byte === 0)) {
        await consumeEndMarker(reader);
        sawEndMarker = true;
        break;
      }
      verifyChecksum(header);

      const size = parseNumber(header.subarray(124, 136));
      if (size === null) throw invalidArchive('文件大小字段无效');
      const mode = parseOctal(header.subarray(100, 108)) ?? 0;
      const type = header[156] === 0 ? '0' : String.fromCharCode(header[156]);

      if (type === 'L' || type === 'K' || type === 'x' || type === 'g') {
        const payload = await readExtension(reader, size);
        await reader.skip(paddingFor(size));
        if (type === 'L') pendingName = decodeUtf8(payload);
        else if (type === 'x') pendingPax = { ...(pendingPax || {}), ...parsePax(payload) };
        continue;
      }

      let name;
      try {
        name = decodeUtf8(header.subarray(0, 100));
        const magic = header.subarray(257, 263).toString('latin1');
        const prefix = decodeUtf8(header.subarray(345, 500));
        if (magic === 'ustar\0' && prefix) name = `${prefix}/${name}`;
      } catch {
        throw unsafeArchive('交付包包含无法识别的文件名，请使用支持的文件名重新打包');
      }
      if (pendingName !== null) {
        name = pendingName;
        pendingName = null;
      }
      let entrySize = size;
      if (pendingPax) {
        if (pendingPax.path) name = pendingPax.path;
        if (pendingPax.size !== undefined) {
          if (!/^[0-9]+$/.test(pendingPax.size)) throw invalidArchive('扩展头文件大小无效');
          const paxSize = Number(pendingPax.size);
          if (!Number.isSafeInteger(paxSize)) throw invalidArchive('扩展头文件大小无效');
          entrySize = paxSize;
        }
        pendingPax = null;
      }

      const isFile = type === '0' || type === '7';
      const isDir = type === '5';
      if (!isFile && !isDir) {
        throw unsafeArchive('交付包包含链接或特殊文件，无法解压，请重新打包后重试');
      }

      const relative = name;
      if (!isSafeEntry(relative)) throw unsafeArchive(`构建物包含非法路径：${relative}`);

      entries += 1;
      if (isFile) files += 1;
      bytes += entrySize;
      if (files > maxFiles || bytes > maxBytes) {
        throw archiveError('交付包解压后体积或文件数量超过限制', 'ARTIFACT_TOO_LARGE');
      }

      const target = path.join(destination, ...relative.split('/'));
      if (isDir) {
        try {
          await fs.mkdir(target, { recursive: true, mode: 0o777 });
        } catch (error) {
          throw extractFailed(relative, error);
        }
        directories.push({ path: target, mode });
        await reader.skip(entrySize);
        await reader.skip(paddingFor(entrySize));
        continue;
      }
      try {
        await fs.mkdir(path.dirname(target), { recursive: true, mode: 0o777 });
      } catch (error) {
        throw extractFailed(relative, error);
      }
      await writeFileEntry(reader, target, relative, entrySize, mode);
      await reader.skip(paddingFor(entrySize));
    }
    if (!sawEndMarker) throw invalidArchive('缺少归档结束标记');
  } catch (error) {
    if (error instanceof DeployError) throw error;
    throw invalidArchive(error.message);
  } finally {
    reader.destroy();
  }

  if (process.platform !== 'win32') {
    for (const directory of directories.reverse()) {
      try {
        await fs.chmod(directory.path, permissionMode(directory.mode, 0o755));
      } catch (error) {
        throw extractFailed(directory.path, error);
      }
    }
  }
  return { entries, files, bytes };
}
