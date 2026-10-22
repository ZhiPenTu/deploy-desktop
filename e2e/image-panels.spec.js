import { expect, test } from '@playwright/test';
import { useDesktopBackend } from './support/desktop.js';

const offlineDir = '/var/lib/deploy/offline';

const dockerImages = [
  {
    reference: `registry.example.com/demo/api@sha256:${'a'.repeat(64)}`,
    id: `sha256:${'b'.repeat(64)}`,
    sizeBytes: 209_715_200,
    exportable: true,
    createdText: '2026-10-01 10:00:00'
  },
  {
    reference: 'demo/worker:latest',
    id: `sha256:${'c'.repeat(64)}`,
    sizeBytes: 1_048_576,
    exportable: false,
    createdText: ''
  }
];

const offlinePackages = [
  {
    name: 'api-1.0.0.tar',
    sizeBytes: 209_715_200,
    modifiedMs: 1_760_000_000_000,
    imageRefs: [`registry.example.com/demo/api@sha256:${'a'.repeat(64)}`]
  },
  {
    name: 'worker-1.0.0.tar',
    sizeBytes: 1_048_576,
    modifiedMs: 1_760_000_000_000,
    imageRefs: []
  }
];

test('镜像与离线包页面按标签分别展示列表状态', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '镜像与离线包' }).click();

  const imagesPanel = page.locator('.image-card', {
    has: page.getByRole('heading', { name: '本机 Docker 镜像' })
  });
  await expect(imagesPanel).toBeVisible();
  await expect(imagesPanel.locator('.card-title-bar .hint')).toContainText('个镜像');
  await expect(imagesPanel.getByRole('button', { name: '拉取镜像' }).first()).toBeVisible();

  // 浏览器不连接 Docker，面板只说明不可用原因与处理方式，不展示虚构镜像。
  await expect(imagesPanel.locator('.error-alert')).toContainText('当前操作暂不可用');
  await expect(imagesPanel.locator('.el-table')).toHaveCount(0);
  await expect(imagesPanel.getByRole('button', { name: '导出离线包' })).toBeDisabled();
  await expect(imagesPanel.getByRole('button', { name: '删除选中镜像' })).toBeDisabled();

  await page.getByRole('tab', { name: '离线包列表' }).click();

  const packagesPanel = page.locator('.image-card', {
    has: page.getByRole('heading', { name: '离线镜像包' })
  });
  await expect(packagesPanel).toBeVisible();
  await expect(packagesPanel.locator('.dir-label')).toHaveText('保存目录');
  await expect(packagesPanel.getByRole('button', { name: '修改目录' })).toBeVisible();
  await expect(packagesPanel.locator('.error-alert')).toContainText('当前操作暂不可用');
  await expect(packagesPanel.getByRole('button', { name: '导入到本机 Docker' })).toBeDisabled();

  await packagesPanel.getByRole('button', { name: '从镜像列表导出' }).click();
  await expect(page.getByRole('heading', { name: '本机 Docker 镜像' })).toBeVisible();

  expect(problems).toEqual([]);
});

test('有数据时展示数量与选择状态，刷新后清空选择', async ({ page }) => {
  await useDesktopBackend(page, { dockerImages, offlineDir, offlinePackages });
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '镜像与离线包' }).click();

  const imagesPanel = page.locator('.image-card', {
    has: page.getByRole('heading', { name: '本机 Docker 镜像' })
  });
  await expect(imagesPanel.locator('.el-table__row')).toHaveCount(2);
  await expect(imagesPanel.locator('.card-title-bar .hint')).toHaveText('2 个镜像 · 共 201.0 MB');
  await expect(imagesPanel.locator('.el-table__row').first()).toContainText('可导出');
  await expect(imagesPanel.locator('.el-table__row').nth(1)).toContainText('仅可删除');
  await expect(imagesPanel.locator('.selection-hint')).toHaveText('已选择 0 个镜像');
  await expect(imagesPanel.getByRole('button', { name: '导出离线包' })).toBeDisabled();

  await imagesPanel.locator('.el-table__row').first().locator('.el-checkbox').click();
  await expect(imagesPanel.locator('.selection-hint')).toHaveText('已选择 1 个镜像');
  await expect(imagesPanel.getByRole('button', { name: '导出离线包' })).toBeEnabled();
  await expect(imagesPanel.getByRole('button', { name: '删除选中镜像' })).toBeEnabled();

  await imagesPanel.getByRole('button', { name: '刷新镜像' }).click();
  await expect(imagesPanel.locator('.selection-hint')).toHaveText('已选择 0 个镜像');
  await expect(imagesPanel.getByRole('button', { name: '导出离线包' })).toBeDisabled();

  await page.getByRole('tab', { name: '离线包列表' }).click();
  const packagesPanel = page.locator('.image-card', {
    has: page.getByRole('heading', { name: '离线镜像包' })
  });
  await expect(packagesPanel.locator('.path-text')).toHaveText(offlineDir);
  await expect(packagesPanel.locator('.card-title-bar .hint')).toHaveText('2 个文件 · 共 201.0 MB');
  await expect(packagesPanel.locator('.el-table__row')).toHaveCount(2);
  await expect(packagesPanel.locator('.el-table__row').first()).toContainText(
    'registry.example.com/demo/api'
  );
  await expect(packagesPanel.locator('.el-table__row').nth(1)).toContainText('未记录对应镜像');
  await expect(packagesPanel.getByRole('button', { name: '导入到本机 Docker' })).toBeDisabled();

  await packagesPanel.locator('.el-table__row').first().locator('.el-checkbox').click();
  await expect(packagesPanel.locator('.selection-hint')).toHaveText('已选择 1 个离线包');
  await expect(packagesPanel.getByRole('button', { name: '导入到本机 Docker' })).toBeEnabled();

  await packagesPanel.getByRole('button', { name: '刷新离线包' }).click();
  await expect(packagesPanel.locator('.selection-hint')).toHaveText('已选择 0 个离线包');

  expect(problems).toEqual([]);
});
