import { expect, test } from '@playwright/test';
import { useDesktopBackend } from './support/desktop.js';

const savedToken = {
  id: 'tok-1',
  name: '云效个人令牌',
  hasToken: true,
  inUse: 0,
  createdAt: 1,
  updatedAt: 1
};

async function openSettings(page) {
  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '设置' }).click();
  await expect(page.locator('.crumb-current')).toHaveText('设置');
}

async function fillFlowSource(dialog, name) {
  await dialog.getByPlaceholder('例如：生产交付流水线').fill(name);
  await dialog.getByPlaceholder('云效组织 ID').fill('org-1');
  await dialog.getByPlaceholder('数字流水线 ID').fill('1001');
}

test('新建制品源时直接选用已保存的令牌', async ({ page }) => {
  await useDesktopBackend(page, { artifactTokens: [savedToken] });
  await openSettings(page);
  await expect(page.locator('.content-panel')).toContainText('云效个人令牌');
  await expect(page.locator('.content-panel')).toContainText('未关联');

  await page.getByRole('button', { name: '新建制品源' }).click();
  const dialog = page.locator('.el-dialog');
  const tokenField = dialog.locator('.el-form-item', { hasText: '访问令牌' });
  await expect(tokenField).toContainText('云效个人令牌');
  await expect(dialog.locator('.el-form-item__label', { hasText: '令牌内容' })).toHaveCount(0);

  await fillFlowSource(dialog, '生产制品源');
  await dialog.getByRole('button', { name: '保存' }).click();

  await expect(page.locator('.el-message__content').last()).toContainText('制品源已保存');
  await expect(page.locator('.content-panel')).toContainText('生产制品源');
  await expect(page.locator('.content-panel')).toContainText('云效个人令牌');
});

test('没有可用令牌时提示新建并校验令牌内容', async ({ page }) => {
  await useDesktopBackend(page);
  await openSettings(page);
  await expect(page.locator('.content-panel')).toContainText('还没有保存的访问令牌');

  await page.getByRole('button', { name: '新建制品源' }).click();
  const dialog = page.locator('.el-dialog');
  await expect(dialog.locator('.el-form-item__label', { hasText: '令牌名称' })).toBeVisible();
  await expect(dialog.locator('.el-form-item__label', { hasText: '令牌内容' })).toBeVisible();

  await fillFlowSource(dialog, '本机制品源');
  await dialog.getByRole('button', { name: '保存' }).click();

  await expect(page.locator('.el-message__content').last()).toContainText('请填写令牌名称和令牌内容');
  await expect(dialog).toBeVisible();
});

test('新建令牌后自动选中并保存制品源', async ({ page }) => {
  await useDesktopBackend(page);
  await openSettings(page);

  await page.getByRole('button', { name: '新建制品源' }).click();
  const dialog = page.locator('.el-dialog');
  await fillFlowSource(dialog, '本机制品源');
  await dialog.getByPlaceholder('例如：云效个人令牌').fill('云效令牌');
  await dialog.getByPlaceholder('云效个人访问令牌').fill('token-value');
  await dialog.getByRole('button', { name: '保存' }).click();

  await expect(page.locator('.el-message__content').last()).toContainText('制品源已保存');
  await expect(page.locator('.content-panel')).toContainText('云效令牌');
  await expect(page.locator('.content-panel')).toContainText('1 个');
});
