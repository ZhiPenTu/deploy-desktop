import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { validateDescriptor } from '../src/bundle.mjs';

// 与 Rust 侧 src-tauri/src/commands/bundle.rs 共用同一份描述符用例，
// 只比较「接受 / 拒绝」，两侧错误文案不同不影响结论。
const here = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = path.resolve(here, '../../fixtures/bundles/cases.json');

function isPlainObject(value) {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function merge(base, patch) {
  if (!isPlainObject(patch)) return structuredClone(patch);
  const result = isPlainObject(base) ? structuredClone(base) : {};
  for (const [key, value] of Object.entries(patch)) {
    result[key] = isPlainObject(value) ? merge(result[key], value) : structuredClone(value);
  }
  return result;
}

const fixture = JSON.parse(await fs.readFile(fixturePath, 'utf8'));

test('bundle 描述符用例表结构完整', () => {
  assert.ok(fixture.cases.length > 0);
  const names = new Set();
  for (const item of fixture.cases) {
    assert.ok(item.name, '用例缺少 name');
    assert.ok(!names.has(item.name), `用例名重复：${item.name}`);
    names.add(item.name);
    assert.ok(item.reason, `用例 ${item.name} 缺少 reason`);
    assert.ok(['ok', 'reject'].includes(item.expect.mcp), `用例 ${item.name} 缺少 mcp 期望`);
  }
});

for (const item of fixture.cases) {
  test(`bundle 描述符用例：${item.name}（MCP）`, () => {
    const descriptor = merge(fixture.base, item.patch);
    if (item.expect.mcp === 'ok') {
      assert.doesNotThrow(() => validateDescriptor(descriptor), item.reason);
      return;
    }
    assert.throws(() => validateDescriptor(descriptor), item.reason);
  });
}

test('两侧结论不同的用例都已注明原因', () => {
  const divergent = fixture.cases.filter((item) => item.expect.rust !== item.expect.mcp);
  for (const item of divergent) {
    assert.ok(item.divergence, `用例 ${item.name} 两侧结论不同，但未记录 divergence`);
  }
});
