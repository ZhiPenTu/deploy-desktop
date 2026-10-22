import { expect, test } from '@playwright/test';

test('运行日志面板展示本机说明、分类筛选与空状态', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '设置' }).click();
  await page.getByRole('tab', { name: '运行日志' }).click();

  await expect(page.locator('.runtime-logs')).toBeVisible();
  await expect(page.locator('.log-overview')).toContainText('日志只保存在本机');
  await expect(page.locator('.log-filters .filter-card')).toHaveCount(4);
  await expect(page.locator('.log-stream .empty-title')).toHaveText('还没有运行日志');

  await page.locator('.log-filters .filter-card', { hasText: '正常' }).click();
  await expect(page.locator('.log-filters .filter-card.is-active')).toContainText('正常');
  await expect(page.locator('.log-stream .empty-title')).toHaveText('当前分类还没有日志');

  expect(problems).toEqual([]);
});
