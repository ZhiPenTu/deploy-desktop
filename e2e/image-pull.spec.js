import { expect, test } from '@playwright/test';

const sha256 = 'a'.repeat(64);

test('拉取镜像弹窗校验摘要并要求完整地址', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '镜像与离线包' }).click();
  await page.getByRole('button', { name: '拉取镜像' }).first().click();

  const dialog = page.locator('.el-dialog', { hasText: '拉取镜像' });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('.dialog-hint')).toContainText('使用 sha256 摘要固定镜像版本');

  const reference = dialog.getByLabel('镜像地址');
  await reference.fill('registry.example.com/demo/app');
  await dialog.getByRole('button', { name: '开始拉取' }).click();
  await expect(page.locator('.el-message', { hasText: '64 位小写 sha256 摘要' })).toBeVisible();

  await reference.fill(`registry.example.com/demo/app@sha256:${sha256}`);
  await expect(dialog.locator('.dialog-hint')).toContainText(
    `拉取地址：registry.example.com/demo/app@sha256:${sha256}`
  );

  await dialog.getByRole('button', { name: '取消' }).click();
  await expect(dialog).toBeHidden();
  expect(problems).toEqual([]);
});
