import { expect, test } from '@playwright/test';
import { filterMcpCategories, mcpCategories, mcpTools } from '../src/features/mcp/catalog.js';

const countOf = (options) =>
  filterMcpCategories(options).reduce((total, category) => total + category.tools.length, 0);

const total = mcpTools.length;
const keywordHits = countOf({ keyword: '日志' });
const logCategory = mcpCategories.find((category) => category.title === '日志');

async function openMcpPage(page) {
  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: 'MCP 服务' }).click();
  await expect(page.locator('.crumb-current')).toHaveText('MCP 服务');
}

async function openMcpCatalog(page) {
  await openMcpPage(page);
  await page.getByRole('tab', { name: '能力目录' }).click();
}

test('MCP 默认显示配置指南并切换各客户端配置', async ({ page }) => {
  await openMcpPage(page);

  const guide = page.locator('.setup-guide');
  await expect(guide).toBeVisible();
  await expect(page.getByRole('tab', { name: '配置指南' })).toHaveAttribute('aria-selected', 'true');
  await expect(guide).toContainText('准备服务文件');
  await expect(guide).toContainText('添加到 AI 助理');
  await expect(guide).toContainText('重启并验证连接');
  await expect(guide.locator('.config-block')).toContainText('[mcp_servers.deploy-desktop]');
  await expect(guide.locator('.config-block')).toContainText('mcp-server/index.mjs');
  await expect(guide.locator('.command-block')).toContainText('node mcp-server/index.mjs --version');

  await guide.getByRole('button', { name: 'Claude Desktop' }).click();
  await expect(guide.locator('.config-block')).toContainText('"mcpServers"');
  await expect(guide).toContainText('设置 > 开发者 > 编辑配置');

  await guide.getByRole('button', { name: 'Claude Code' }).click();
  await expect(guide).toContainText('项目根目录的 .mcp.json');
  await expect(guide.locator('.config-block')).toContainText('"deploy-desktop"');

  await guide.getByRole('button', { name: 'Cursor' }).click();
  await expect(guide).toContainText('.cursor/mcp.json');
  await expect(guide.locator('.config-block')).toContainText('"deploy-desktop"');
});

test('MCP 能力目录按关键词筛选并显示匹配数量', async ({ page }) => {
  await openMcpCatalog(page);
  const count = page.locator('.result-count');
  const search = page.getByRole('textbox', { name: '搜索 MCP 能力' });

  await expect(count).toHaveText(`${total} 项能力`);
  await expect(page.locator('.capability-item')).toHaveCount(total);

  await search.fill('日志');
  await expect(count).toHaveText(`${keywordHits} 项能力`);
  await expect(page.locator('.capability-item')).toHaveCount(keywordHits);

  await search.fill('不存在的关键词');
  await expect(count).toHaveText('0 项能力');
  await expect(page.locator('.capability-results')).toContainText('没有匹配的 MCP 能力');

  await page.getByRole('button', { name: '重置筛选' }).last().click();
  await expect(count).toHaveText(`${total} 项能力`);
  await expect(page.locator('.capability-item')).toHaveCount(total);
});

test('MCP 能力目录按分类筛选，并能回到全部能力', async ({ page }) => {
  await openMcpCatalog(page);
  const count = page.locator('.result-count');

  await page.locator('.category-button', { hasText: logCategory.title }).first().click();
  await expect(count).toHaveText(`${logCategory.tools.length} 项能力`);
  await expect(page.locator('.capability-item')).toHaveCount(logCategory.tools.length);

  await page.locator('.category-button', { hasText: '全部能力' }).click();
  await expect(count).toHaveText(`${total} 项能力`);
});

test('MCP 能力说明不出现工具名或开发说明', async ({ page }) => {
  await openMcpCatalog(page);
  const text = await page.locator('.capability-panel').innerText();

  for (const word of ['mcp__', 'deploy_', 'TODO', 'FIXME', '接口', '模拟']) {
    expect(text, `能力说明出现了开发信息「${word}」`).not.toContain(word);
  }
});
