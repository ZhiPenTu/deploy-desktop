import assert from 'node:assert/strict';
import test from 'node:test';
import { formatDockerCreated, formatImageId } from '../src/shared/format.js';

test('镜像 ID 去掉 sha256 前缀后保留前 12 位', () => {
  assert.equal(formatImageId(`sha256:${'a'.repeat(64)}`), 'a'.repeat(12));
  assert.equal(formatImageId('b'.repeat(64)), 'b'.repeat(12));
});

test('镜像 ID 缺失时显示占位符', () => {
  assert.equal(formatImageId(''), '-');
  assert.equal(formatImageId(undefined), '-');
});

test('镜像创建时间去掉秒和时区后缀', () => {
  assert.equal(formatDockerCreated('2026-10-28 20:01:18 +0800 CST'), '2026-10-28 20:01');
  assert.equal(formatDockerCreated('2026-10-19 23:23:30 +0800 CST'), '2026-10-19 23:23');
});

test('镜像创建时间缺失或格式未知时保留可读内容', () => {
  assert.equal(formatDockerCreated(''), '-');
  assert.equal(formatDockerCreated(undefined), '-');
  assert.equal(formatDockerCreated('刚刚创建'), '刚刚创建');
});
