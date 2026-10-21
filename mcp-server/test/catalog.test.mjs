import assert from 'node:assert/strict';
import test from 'node:test';
import { mcpCategories } from '../../src/features/mcp/catalog.js';
import { registerTools } from '../src/tools.mjs';

test('界面能力目录完整覆盖实际注册工具，且只读与确认标记保持一致', () => {
  const registered = new Map();
  registerTools({ registerTool: (name, definition) => registered.set(name, definition) });
  const displayed = mcpCategories.flatMap((category) => category.tools);
  assert.equal(new Set(mcpCategories.map((category) => category.id)).size, mcpCategories.length);
  assert.equal(new Set(displayed.map((tool) => tool.id)).size, displayed.length);
  assert.deepEqual(displayed.map((tool) => tool.id).sort(), [...registered.keys()].sort());
  for (const tool of displayed) {
    const definition = registered.get(tool.id);
    assert.equal(tool.access === 'read', definition.annotations.readOnlyHint, `${tool.id} 只读标记不一致`);
    assert.equal(Boolean(tool.confirmation), Boolean(definition.inputSchema.confirm), `${tool.id} 确认标记不一致`);
  }
});
