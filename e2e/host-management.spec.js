import { expect, test } from '@playwright/test';
import { remoteHost, sshCredential, useDesktopBackend } from './support/desktop.js';

async function openHosts(page) {
  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '主机管理' }).click();
  await expect(page.locator('.crumb-current')).toHaveText('主机管理');
}

test('保存主机后列表、计数与提示同步更新', async ({ page }) => {
  await useDesktopBackend(page);
  await openHosts(page);
  await expect(page.locator('.content-panel')).toContainText('0 台主机');

  await page.getByRole('button', { name: '新增主机' }).click();
  const dialog = page.locator('.el-dialog');
  await dialog.getByPlaceholder('例如：本机').fill('本机目标');
  await dialog.getByPlaceholder('例如：/opt/deploy').fill('/opt/demo');
  await dialog.getByRole('button', { name: '保存' }).click();

  await expect(page.locator('.el-message__content').last()).toContainText('主机已保存');
  await expect(page.locator('.content-panel')).toContainText('1 台主机');
  await expect(page.locator('.host-card')).toHaveCount(1);
  await expect(page.locator('.host-card')).toContainText('本机目标');
  await expect(page.locator('.host-card')).toContainText('/opt/demo');
  await expect(page.locator('.empty-state')).toHaveCount(0);
});

test('已保存的远程主机显示地址、凭证与部署目录', async ({ page }) => {
  await useDesktopBackend(page, { hosts: [remoteHost], credentials: [sshCredential] });
  await openHosts(page);

  const card = page.locator('.host-card');
  await expect(card).toHaveCount(1);
  await expect(card).toContainText('测试服务器');
  await expect(card).toContainText('192.0.2.10:2222');
  await expect(card).toContainText('部署账号');
  await expect(card).toContainText('/opt/demo');
  await expect(card.getByRole('button', { name: '测试连接' })).toBeVisible();
});

test('删除主机时取消确认不会移除主机', async ({ page }) => {
  await useDesktopBackend(page, { hosts: [remoteHost], credentials: [sshCredential] });
  await openHosts(page);

  await page.locator('.host-card').getByRole('button', { name: '删除' }).click();
  const confirm = page.locator('.el-message-box');
  await expect(confirm).toContainText('确认删除主机');
  await confirm.getByRole('button', { name: '取消' }).click();

  await expect(page.locator('.host-card')).toHaveCount(1);
  await expect(page.locator('.content-panel')).toContainText('1 台主机');
});

test('确认删除后主机列表同步清空', async ({ page }) => {
  await useDesktopBackend(page, { hosts: [remoteHost], credentials: [sshCredential] });
  await openHosts(page);

  await page.locator('.host-card').getByRole('button', { name: '删除' }).click();
  await page.locator('.el-message-box').getByRole('button', { name: '删除' }).click();

  await expect(page.locator('.el-message__content').last()).toContainText('主机已删除');
  await expect(page.locator('.content-panel')).toContainText('0 台主机');
  await expect(page.locator('.empty-state')).toContainText('还没有保存的主机');
});
