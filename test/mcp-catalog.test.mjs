import assert from 'node:assert/strict';
import { test } from 'node:test';
import { filterMcpCategories, mcpCategories, mcpTools } from '../src/features/mcp/catalog.js';

const flatten = (categories) => categories.flatMap((category) => category.tools);

test('能力目录默认返回全部条目且工具标识唯一', () => {
  const visible = flatten(filterMcpCategories());
  assert.equal(visible.length, mcpTools.length);
  const ids = mcpTools.map((tool) => tool.id);
  assert.equal(new Set(ids).size, ids.length);
});

test('关键词只匹配展示文案', () => {
  const visible = flatten(filterMcpCategories({ keyword: '日志' }));
  assert.ok(visible.length > 0 && visible.length < mcpTools.length);
  for (const category of filterMcpCategories({ keyword: '日志' })) {
    const content = `${category.title} ${category.description}`;
    for (const tool of category.tools) {
      assert.ok(`${content} ${tool.title} ${tool.description} ${tool.requirement || ''}`.includes('日志'));
    }
  }
  assert.deepEqual(filterMcpCategories({ keyword: 'deploy_logs' }), []);
});

test('分类、操作类型与关键词可以叠加过滤', () => {
  const logCategory = mcpCategories.find((category) => category.title === '日志');
  assert.ok(logCategory);
  assert.deepEqual(
    filterMcpCategories({ category: logCategory.id }).map((category) => category.id),
    [logCategory.id]
  );

  for (const tool of flatten(filterMcpCategories({ access: 'read' }))) {
    assert.equal(tool.access, 'read');
  }
  for (const tool of flatten(filterMcpCategories({ access: 'confirm' }))) {
    assert.ok(tool.confirmation);
  }
  assert.deepEqual(filterMcpCategories({ keyword: '不存在的关键词' }), []);
});
