import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import zlib from 'node:zlib';
import { isSafeEntry, safeExtract } from '../src/tar-extract.mjs';
import { makeTempDir } from './helpers.mjs';

function octal(value, width) {
  return `${Math.max(0, value).toString(8).padStart(width - 1, '0')}\0`;
}

function header({ name, size = 0, mode = 0o644, type = '0', linkname = '' }) {
  const block = Buffer.alloc(512);
  block.set(Buffer.from(name, 'utf8').subarray(0, 100), 0);
  block.write(octal(mode, 8), 100, 8, 'latin1');
  block.write(octal(0, 8), 108, 8, 'latin1');
  block.write(octal(0, 8), 116, 8, 'latin1');
  block.write(octal(size, 12), 124, 12, 'latin1');
  block.write(octal(0, 12), 136, 12, 'latin1');
  block.write('        ', 148, 8, 'latin1');
  block.write(type, 156, 1, 'latin1');
  block.set(Buffer.from(linkname, 'utf8').subarray(0, 100), 157);
  block.write('ustar  \0', 257, 8, 'latin1');
  const checksum = block.reduce((sum, byte) => sum + byte, 0);
  block.write(`${checksum.toString(8).padStart(6, '0')}\0 `, 148, 8, 'latin1');
  return block;
}

function pad(data) {
  const remainder = data.length % 512;
  return remainder === 0 ? data : Buffer.concat([data, Buffer.alloc(512 - remainder)]);
}

function entry(name, data = '', options = {}) {
  const payload = Buffer.from(data);
  return Buffer.concat([header({ name, size: payload.length, ...options }), pad(payload)]);
}

function longNameEntry(name) {
  const payload = Buffer.from(`${name}\0`, 'utf8');
  return Buffer.concat([
    header({ name: '././@LongLink', size: payload.length, type: 'L' }),
    pad(payload)
  ]);
}

function paxRecord(key, value) {
  const body = ` ${key}=${value}\n`;
  let length = Buffer.byteLength(body) + 1;
  while (Buffer.byteLength(`${length}${body}`) !== length) {
    length = Buffer.byteLength(`${length}${body}`);
  }
  return Buffer.from(`${length}${body}`);
}

function paxEntry(records) {
  const payload = Buffer.concat(records.map(([key, value]) => paxRecord(key, value)));
  return Buffer.concat([header({ name: 'PaxHeader', size: payload.length, type: 'x' }), pad(payload)]);
}

function tarGz(parts, { eof = true } = {}) {
  return zlib.gzipSync(Buffer.concat([...parts, ...(eof ? [Buffer.alloc(1024)] : [])]));
}

async function extractBuffer(root, buffer) {
  const archive = path.join(root, 'bundle.tgz');
  await fs.writeFile(archive, buffer);
  return safeExtract(archive, path.join(root, 'out'));
}

test('解包中文文件名和 GNU 长文件名', async () => {
  const root = await makeTempDir();
  try {
    const longName = `app/gateway-classes/${'a'.repeat(80)}/Service.class`;
    assert.ok(Buffer.byteLength(longName) > 100);
    const result = await extractBuffer(
      root,
      tarGz([
        entry('app/frontend-dist/sound/网管告警.wav', 'wav'),
        longNameEntry(longName),
        entry(longName.slice(0, 100), 'class')
      ])
    );
    assert.equal(result.files, 2);
    const out = path.join(root, 'out');
    assert.equal(await fs.readFile(path.join(out, 'app/frontend-dist/sound/网管告警.wav'), 'utf8'), 'wav');
    assert.equal(await fs.readFile(path.join(out, longName), 'utf8'), 'class');
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('拒绝父目录跳转且不写出目标目录', async () => {
  const root = await makeTempDir();
  try {
    await assert.rejects(extractBuffer(root, tarGz([entry('../escape.txt', 'x')])), {
      code: 'ARTIFACT_ARCHIVE_UNSAFE'
    });
    await assert.rejects(fs.access(path.join(root, 'escape.txt')));
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('拒绝链接、设备文件与稀疏条目', async () => {
  const root = await makeTempDir();
  try {
    for (const type of ['1', '2', '3', '4', '6', 'S']) {
      await assert.rejects(
        extractBuffer(root, tarGz([entry('app/link', '', { type, linkname: '/etc/passwd' })])),
        { code: 'ARTIFACT_ARCHIVE_UNSAFE' },
        `type ${type}`
      );
    }
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('拒绝 Windows 保留设备名和非法字符', async () => {
  const root = await makeTempDir();
  try {
    for (const name of [
      'app/NUL',
      'app/con.txt',
      'app/COM¹.txt',
      'app/LPT¹.txt',
      'app/file.',
      'app/file ',
      'app/a:b',
      'app/a\\b',
      'app/a*b',
      'app/a?b',
      'app/a|b',
      'app/a<b',
      'app/a>b',
      'app/a"b',
      'app/a\u0001b'
    ]) {
      await assert.rejects(extractBuffer(root, tarGz([entry(name, 'x')])), {
        code: 'ARTIFACT_ARCHIVE_UNSAFE'
      }, name);
    }
    assert.equal(isSafeEntry('app/common.txt'), true);
    assert.equal(isSafeEntry('/etc/passwd'), false);
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('损坏或截断的压缩包给出可识别的错误', async () => {
  const root = await makeTempDir();
  try {
    const archive = path.join(root, 'broken.tgz');
    await fs.writeFile(archive, 'not a tarball');
    await assert.rejects(safeExtract(archive, path.join(root, 'out')), {
      code: 'ARTIFACT_ARCHIVE_INVALID'
    });

    const truncated = zlib.gzipSync(Buffer.concat([header({ name: 'app/a.txt', size: 10 })]));
    await assert.rejects(extractBuffer(root, truncated), { code: 'ARTIFACT_ARCHIVE_INVALID' });

    const missing = path.join(root, 'missing.tgz');
    await assert.rejects(safeExtract(missing, path.join(root, 'out-missing')), {
      code: 'ARTIFACT_ARCHIVE_INVALID'
    });
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('校验结束标记、校验和与 gzip 尾部', async () => {
  const root = await makeTempDir();
  try {
    await assert.rejects(extractBuffer(root, tarGz([entry('app/a.txt', 'x')], { eof: false })), {
      code: 'ARTIFACT_ARCHIVE_INVALID'
    });
    await assert.rejects(
      extractBuffer(root, zlib.gzipSync(Buffer.concat([entry('app/a.txt', 'x'), Buffer.alloc(512)]))),
      { code: 'ARTIFACT_ARCHIVE_INVALID' }
    );
    const complete = tarGz([entry('app/a.txt', 'x')]);
    await assert.rejects(extractBuffer(root, complete.subarray(0, complete.length - 4)), {
      code: 'ARTIFACT_ARCHIVE_INVALID'
    });
    const tampered = entry('app/a.txt', 'x');
    tampered[4] ^= 0x01;
    await assert.rejects(extractBuffer(root, tarGz([tampered])), {
      code: 'ARTIFACT_ARCHIVE_INVALID'
    });
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('PAX 扩展头覆盖路径与大小', async () => {
  const root = await makeTempDir();
  try {
    const data = Buffer.from('hello');
    const archive = tarGz([
      paxEntry([
        ['path', 'app/中文名.txt'],
        ['size', String(data.length)]
      ]),
      Buffer.concat([header({ name: 'app/placeholder.txt', size: 0 }), pad(data)])
    ]);
    const result = await extractBuffer(root, archive);
    assert.equal(result.files, 1);
    assert.equal(await fs.readFile(path.join(root, 'out/app/中文名.txt'), 'utf8'), 'hello');
    await assert.rejects(
      extractBuffer(root, tarGz([paxEntry([['size', '1e3']]), entry('app/x', '')])),
      { code: 'ARTIFACT_ARCHIVE_INVALID' }
    );
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('超过声明大小时拒绝继续解包', async () => {
  const root = await makeTempDir();
  try {
    const archive = path.join(root, 'bundle.tgz');
    await fs.writeFile(archive, tarGz([entry('app/a.txt', 'hello')]));
    await assert.rejects(safeExtract(archive, path.join(root, 'out'), { maxBytes: 4 }), {
      code: 'ARTIFACT_TOO_LARGE'
    });
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test('解包后不放大组和其他用户的写权限', { skip: process.platform === 'win32' }, async () => {
  const root = await makeTempDir();
  try {
    await extractBuffer(
      root,
      tarGz([
        entry('app', '', { type: '5', mode: 0o777 }),
        entry('app/config.env', 'x', { mode: 0o666 })
      ])
    );
    const out = path.join(root, 'out');
    assert.equal((await fs.stat(path.join(out, 'app/config.env'))).mode & 0o777, 0o644);
    assert.equal((await fs.stat(path.join(out, 'app'))).mode & 0o777, 0o755);
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});
