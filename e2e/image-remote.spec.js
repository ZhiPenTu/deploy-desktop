import { expect, test } from '@playwright/test';
import { remoteHost, sshCredential, useDesktopBackend } from './support/desktop.js';

const project = {
  id: 'project-image-remote',
  name: '测试项目',
  hostId: remoteHost.id,
  installDir: '/opt/demo',
  sourceId: ''
};

const offlineDir = '/var/lib/deploy/offline';

const offlinePackages = [
  {
    name: 'web-1.2.0.tar',
    sizeBytes: 2048,
    modifiedMs: 1_760_000_000_000,
    imageRefs: ['demo/web:1.2.0']
  },
  {
    name: 'api-1.2.0.tar',
    sizeBytes: 4096,
    modifiedMs: 1_760_000_000_000,
    imageRefs: ['demo/api:1.2.0']
  }
];

const remoteImageBundle = {
  exists: true,
  images_dir: '/opt/demo/bundle/images',
  total_bytes: 2048,
  free_kb: 10_485_760,
  docker_running: true,
  issues: ['目标机镜像包与本机离线包不一致'],
  expectations: [
    { file_name: 'web-1.2.0.tar', image_ref: 'demo/web:1.2.0' },
    { file_name: 'api-1.2.0.tar', image_ref: 'demo/api:1.2.0' }
  ],
  files: [
    { name: 'web-1.2.0.tar', size_bytes: 2048, modified_ms: 1_760_000_000_000, image_ref: 'demo/web:1.2.0' }
  ],
  docker_images: ['demo/web:1.2.0']
};

test('远程分发页展示离线包来源、目标机镜像包状态与操作入口', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await useDesktopBackend(page, {
    hosts: [remoteHost],
    projects: [project],
    activeProjectId: project.id,
    offlineDir,
    offlinePackages,
    remoteImageBundle
  });

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '镜像与离线包' }).click();
  await page.getByRole('tab', { name: '远程分发' }).click();

  const source = page.locator('.remote-card', { has: page.getByRole('heading', { name: '离线包来源' }) });
  await expect(source).toContainText(offlineDir);
  await expect(source).toContainText('2 个离线包');

  const bundle = page.locator('.remote-card', { has: page.getByRole('heading', { name: '分发目标' }) });
  await expect(bundle.locator('.dir-input-row input')).toHaveValue('/opt/demo/bundle');
  await expect(bundle).toContainText('Docker 已运行');
  await expect(bundle).toContainText('/opt/demo/bundle/images');
  await expect(bundle.locator('.issue-chip')).toHaveText('目标机镜像包与本机离线包不一致');

  const rows = bundle.locator('.image-table .el-table__row');
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText('web-1.2.0.tar');
  await expect(rows.first()).toContainText('已导入');
  await expect(rows.nth(1)).toContainText('api-1.2.0.tar');
  await expect(rows.nth(1)).toContainText('缺失');

  const actions = page.locator('.remote-card', { has: page.getByRole('heading', { name: '操作' }) });
  await expect(actions.getByRole('button', { name: '上传缺失离线包' })).toBeEnabled();
  await expect(actions.getByRole('button', { name: '仅导入未加载镜像' })).toBeEnabled();
  await expect(actions.getByRole('button', { name: '删除选中镜像包' })).toBeDisabled();

  expect(problems).toEqual([]);
});

test('目标机镜像目录未就绪时给出可操作提示', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await useDesktopBackend(page, {
    hosts: [remoteHost],
    projects: [project],
    activeProjectId: project.id,
    offlineDir,
    offlinePackages,
    remoteImageBundle: {
      ...remoteImageBundle,
      exists: false,
      files: [],
      total_bytes: 0,
      issues: []
    }
  });

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '镜像与离线包' }).click();
  await page.getByRole('tab', { name: '远程分发' }).click();

  const bundle = page.locator('.remote-card', { has: page.getByRole('heading', { name: '分发目标' }) });
  await expect(bundle.locator('.card-title-bar .hint')).toHaveText('镜像目录未就绪');
  const rows = bundle.locator('.image-table .el-table__row');
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText('缺失');

  const actions = page.locator('.remote-card', { has: page.getByRole('heading', { name: '操作' }) });
  await expect(actions.getByRole('button', { name: '仅导入未加载镜像' })).toBeDisabled();

  expect(problems).toEqual([]);
});

test('分发主机可在页面中切换，检测与上传使用所选主机', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));
  const standbyHost = {
    ...remoteHost,
    id: 'host-standby',
    name: '备用服务器',
    address: '192.0.2.20',
    port: 22
  };

  await useDesktopBackend(page, {
    hosts: [remoteHost, standbyHost],
    credentials: [sshCredential],
    projects: [project],
    activeProjectId: project.id,
    offlineDir,
    offlinePackages,
    remoteImageBundle
  });

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '镜像与离线包' }).click();

  await page.getByRole('tab', { name: '远程分发' }).click();

  const bundle = page.locator('.remote-card', { has: page.getByRole('heading', { name: '分发目标' }) });
  const select = bundle.locator('.el-form-item', { hasText: '分发主机' }).locator('.el-select');
  await expect(select).toContainText('测试服务器 · 192.0.2.10');

  await select.click();
  await page.getByRole('option', { name: '备用服务器 · 192.0.2.20' }).click();

  await expect(select).toContainText('备用服务器 · 192.0.2.20');

  const inspections = await page.evaluate(() =>
    window.__deployTestCalls
      .filter((call) => call.command === 'inspect_remote_image_bundle')
      .map((call) => call.args)
  );
  expect(inspections.at(-1)).toMatchObject({
    host: '192.0.2.20',
    port: 22,
    user: 'deploy',
    keyPath: sshCredential.privateKeyPath
  });
  expect(problems).toEqual([]);
});
