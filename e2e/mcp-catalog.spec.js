import { expect, test } from '@playwright/test';

const total = 40;

async function openMcpPage(page) {
  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: 'MCP 服务' }).click();
  await expect(page.locator('.crumb-current')).toHaveText('MCP 服务');
}

test('MCP 能力目录按关键词筛选并显示匹配数量', async ({ page }) => {
  await openMcpPage(page);
  const count = page.locator('.result-count');
  const search = page.getByRole('textbox', { name: '搜索 MCP 能力' });

  await expect(count).toHaveText(`${total} 项能力`);
  await expect(page.locator('.capability-item')).toHaveCount(total);

  await search.fill('日志');
  await expect(count).toHaveText('9 项能力');
  await expect(page.locator('.capability-item')).toHaveCount(9);

  await search.fill('不存在的关键词');
  await expect(count).toHaveText('0 项能力');
  await expect(page.locator('.capability-results')).toContainText('没有匹配的 MCP 能力');

  await page.getByRole('button', { name: '重置筛选' }).last().click();
  await expect(count).toHaveText(`${total} 项能力`);
  await expect(page.locator('.capability-item')).toHaveCount(total);
});

test('MCP 能力目录按分类筛选，并能回到全部能力', async ({ page }) => {
  await openMcpPage(page);
  const count = page.locator('.result-count');

  await page.locator('.category-button', { hasText: '日志' }).first().click();
  await expect(count).toHaveText('2 项能力');
  await expect(page.locator('.capability-item')).toHaveCount(2);

  await page.locator('.category-button', { hasText: '全部能力' }).click();
  await expect(count).toHaveText(`${total} 项能力`);
});

test('MCP 能力说明不出现工具名或开发说明', async ({ page }) => {
  await openMcpPage(page);
  const text = await page.locator('.capability-panel').innerText();

  for (const word of ['mcp__', 'deploy_', 'TODO', 'FIXME', '接口', '模拟']) {
    expect(text, `能力说明出现了开发信息「${word}」`).not.toContain(word);
  }
});
