import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import test from 'node:test';
import os from 'node:os';
import path from 'node:path';
import {
  clamp,
  compareVersions,
  expandHome,
  formatBytes,
  isArchiveFile,
  parseBoolean,
  sanitizeFileName,
  shellQuote,
  unique
} from '../src/util.mjs';

test('expandHome 展开 ~ 与 ~/ 路径', () => {
  assert.equal(expandHome('~'), os.homedir());
  assert.equal(expandHome('~/bundle'), path.join(os.homedir(), 'bundle'));
  assert.equal(expandHome('/opt/deploy'), '/opt/deploy');
  assert.equal(expandHome(undefined), undefined);
});

test('shellQuote 处理单引号', () => {
  assert.equal(shellQuote('plain'), "'plain'");
  assert.equal(shellQuote("a'b"), "'a'\\''b'");
});

test('formatBytes 使用可读单位', () => {
  assert.equal(formatBytes(0), '0 B');
  assert.equal(formatBytes(1024), '1.0 KB');
  assert.equal(formatBytes(1024 ** 2), '1.0 MB');
  assert.equal(formatBytes(1024 ** 3), '1.00 GB');
});

test('parseBoolean 支持常见真值', () => {
  for (const value of [true, '1', 'true', 'YES', 'on']) assert.equal(parseBoolean(value), true);
  for (const value of [false, '0', 'false', 'off', null]) assert.equal(parseBoolean(value), false);
});

test('compareVersions 与桌面端共用 fixtures/versions.json', async () => {
  const raw = await fs.readFile(new URL('../../fixtures/versions.json', import.meta.url), 'utf8');
  const { cases } = JSON.parse(raw);
  assert.ok(cases.length > 0);
  for (const item of cases) {
    assert.equal(
      compareVersions(item.left, item.right),
      item.expected,
      `${item.left} ? ${item.right}`
    );
    assert.equal(
      compareVersions(item.right, item.left),
      item.expected === 0 ? 0 : -item.expected,
      `${item.right} ? ${item.left}`
    );
  }
});

test('compareVersions 比较语义版本', () => {
  assert.equal(compareVersions('0.1.2', '0.1.3'), -1);
  assert.equal(compareVersions('v1.0.0', '1.0.0'), 0);
  assert.equal(compareVersions('2.0', '1.9.9'), 1);
});

test('clamp 与 unique 行为稳定', () => {
  assert.equal(clamp(5, 1, 3), 3);
  assert.equal(clamp('x', 2, 3), 2);
  assert.deepEqual(unique(['a', 'b', 'a']), ['a', 'b']);
});

test('sanitizeFileName 拒绝路径穿越', () => {
  assert.equal(sanitizeFileName('image.tar'), 'image.tar');
  assert.throws(() => sanitizeFileName('../image.tar'), /非法的文件名/);
  assert.throws(() => sanitizeFileName('dir/image.tar'), /非法的文件名/);
  assert.throws(() => sanitizeFileName(''), /非法的文件名/);
});

test('isArchiveFile 识别 tar 与压缩 tar', () => {
  assert.equal(isArchiveFile('a.tar'), true);
  assert.equal(isArchiveFile('a.tgz'), true);
  assert.equal(isArchiveFile('a.tar.gz'), true);
  assert.equal(isArchiveFile('a.zip'), false);
});
