import assert from 'node:assert/strict';
import test from 'node:test';
import { formatImageId } from '../src/shared/format.js';

test('镜像 ID 去掉 sha256 前缀后保留前 12 位', () => {
  assert.equal(formatImageId(`sha256:${'a'.repeat(64)}`), 'a'.repeat(12));
  assert.equal(formatImageId('b'.repeat(64)), 'b'.repeat(12));
});

test('镜像 ID 缺失时显示占位符', () => {
  assert.equal(formatImageId(''), '-');
  assert.equal(formatImageId(undefined), '-');
});
