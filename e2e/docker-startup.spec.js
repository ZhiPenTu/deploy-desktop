import { expect, test } from '@playwright/test';
import { useDesktopBackend } from './support/desktop.js';

for (const [os, label] of [['windows', 'Windows'], ['linux', 'Linux'], ['macos', 'macOS']]) {
  test(`${label} 启动缺少 Docker 时提供安装入口，只有重新检测到安装才关闭提示`, async ({ page }) => {
    const problems = [];
    page.on('pageerror', (error) => problems.push(error.message));
    page.on('console', (message) => {
      if (['error', 'warning'].includes(message.type())) problems.push(message.text());
    });
    await useDesktopBackend(page, { dockerInstallation: { os, installed: false, version: null } });
    await page.goto('/');
    const dialog = page.getByRole('dialog', { name: '未检测到 Docker' });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(`当前 ${label} 电脑尚未安装 Docker`);
    await expect(dialog).toContainText('官方安装页面');
    await dialog.getByRole('button', { name: '安装 Docker', exact: true }).click();
    await expect(dialog).toContainText('已请求打开 Docker 官方安装页面');
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: '重新检测' }).click();
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('button', { name: '重新检测' })).toBeEnabled();
    const calls = await page.evaluate(() => window.__deployTestCalls);
    expect(calls.filter((item) => item.command === 'open_docker_installation')).toHaveLength(1);
    expect(calls.filter((item) => item.command === 'check_docker_installation')).toHaveLength(2);
    expect(calls.some((item) => item.command === 'check_local_env' || item.command === 'check_remote_env')).toBe(false);

    await page.evaluate(() => {
      const original = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === 'check_docker_installation') return { installed: true, version: 'Docker version 27.0.0' };
        return original(command, args);
      };
      window.dispatchEvent(new Event('focus'));
    });
    await expect(dialog).not.toBeVisible();
    await expect(page.locator('.el-message__content').last()).toHaveText('已检测到 Docker。');
    expect(problems).toEqual([]);
  });
}

test('稍后处理不打开安装页面，切换页面后不重复弹出，下次启动仍会检查', async ({ page }) => {
  await useDesktopBackend(page, { dockerInstallation: { os: 'linux', installed: false } });
  await page.goto('/');
  const dialog = page.getByRole('dialog', { name: '未检测到 Docker' });
  await dialog.getByRole('button', { name: '稍后处理' }).click();
  await page.locator('.nav-menu .nav-item', { hasText: '主机管理' }).click();
  await expect(page.locator('.crumb-current')).toHaveText('主机管理');
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect(dialog).not.toBeVisible();
  const calls = await page.evaluate(() => window.__deployTestCalls);
  expect(calls.filter((item) => item.command === 'check_docker_installation')).toHaveLength(1);
  expect(calls.filter((item) => item.command === 'open_docker_installation')).toHaveLength(0);
  await page.reload();
  await expect(dialog).toBeVisible();
});

test('已安装 Docker 时不弹出缺失提示，浏览器预览不执行安装检查', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('.app-sidebar')).toBeVisible();
  await expect(page.getByRole('dialog')).not.toBeVisible();
  expect(await page.evaluate(() => window.isTauri)).toBeFalsy();

  await useDesktopBackend(page);
  await page.reload();
  await expect(page.locator('.app-sidebar')).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.__deployTestCalls.filter((item) =>
    item.command === 'check_docker_installation').length)).toBe(1);
  await expect(page.getByRole('dialog')).not.toBeVisible();
});

test('检测失败不会误报未安装，重试后按真实结果展示', async ({ page }) => {
  await useDesktopBackend(page, { dockerInstallation: null });
  await page.goto('/');
  const failedDialog = page.getByRole('dialog', { name: '无法检查 Docker' });
  await expect(failedDialog).toBeVisible();
  await expect(failedDialog).toContainText('无法读取 Docker 安装状态，请重新检测。');
  await expect(failedDialog.getByRole('button', { name: '安装 Docker', exact: true })).toHaveCount(0);
  await page.evaluate(() => {
    const original = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === 'check_docker_installation') return { os: 'macos', installed: false };
      return original(command, args);
    };
  });
  await failedDialog.getByRole('button', { name: '重新检测' }).click();
  const missingDialog = page.getByRole('dialog', { name: '未检测到 Docker' });
  await expect(missingDialog).toBeVisible();
  await expect(missingDialog).toContainText('macOS');
});

test('打开安装页面期间禁止重复操作，失败后显示原因并可重试', async ({ page }) => {
  await useDesktopBackend(page, { dockerInstallation: { os: 'windows', installed: false } });
  await page.goto('/');
  const dialog = page.getByRole('dialog', { name: '未检测到 Docker' });
  await expect(dialog).toBeVisible();
  await page.evaluate(() => {
    const original = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === 'open_docker_installation') {
        return new Promise((resolve, reject) => { window.__rejectDockerOpen = reject; });
      }
      return original(command, args);
    };
  });
  const install = dialog.getByRole('button', { name: '安装 Docker', exact: true });
  await install.click();
  await expect(install).toBeDisabled();
  await expect(dialog.getByRole('button', { name: '重新检测' })).toBeDisabled();
  await expect(dialog.getByRole('button', { name: '稍后处理' })).toBeDisabled();
  await page.evaluate(() => window.__rejectDockerOpen('无法打开 Docker 官方安装页面，请检查默认浏览器后重试。'));
  await expect(dialog).toContainText('请检查默认浏览器后重试');
  await expect(dialog).not.toContainText('已请求打开');
  await expect(install).toBeEnabled();
  await page.evaluate(() => {
    const original = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === 'open_docker_installation') return null;
      return original(command, args);
    };
  });
  await install.click();
  await expect(dialog).toContainText('已请求打开 Docker 官方安装页面');
  await expect(dialog).not.toContainText('请检查默认浏览器后重试');
});
