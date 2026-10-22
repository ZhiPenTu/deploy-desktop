import { expect, test } from '@playwright/test';
import { useDesktopBackend } from './support/desktop.js';

const flowSource = {
  id: 'src-1',
  name: '生产交付流水线',
  type: 'flow',
  orgId: 'org-1',
  pipelineId: 1001,
  artifactName: 'app',
  lastVersion: 'v1.2.0',
  lastBundleDir: ''
};

const releases = [
  {
    runId: 2001,
    version: 'v1.2.0',
    commitShort: 'abc1234',
    commitTitle: '修复登录重定向',
    branch: 'main',
    sizeBytes: 1_048_576,
    builtAt: 1_760_000_000_000,
    localDir: '/opt/app',
    downloadUrl: 'https://example.com/app-1.2.0.tgz',
    runUrl: 'https://example.com/runs/2001'
  },
  {
    runId: 1999,
    version: 'v1.1.0',
    commitShort: 'def5678',
    commitTitle: '新增导出入口',
    branch: 'main',
    sizeBytes: 524_288,
    builtAt: 1_759_900_000_000,
    localDir: '',
    downloadUrl: 'https://example.com/app-1.1.0.tgz',
    runUrl: ''
  }
];

test('没有制品源时提示前往设置', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '制品中心' }).click();

  const empty = page.locator('.empty-panel');
  await expect(empty).toBeVisible();
  await expect(empty).toContainText('还没有可用的制品源');
  await expect(empty.getByRole('button', { name: '前往设置' })).toBeVisible();
  await expect(page.locator('.source-card')).toHaveCount(0);

  expect(problems).toEqual([]);
});

test('制品源列表进入版本页后展示概览与版本明细', async ({ page }) => {
  await useDesktopBackend(page, { artifactSources: [flowSource], artifactReleases: releases });
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '制品中心' }).click();

  const sourceCard = page.locator('.source-card');
  await expect(sourceCard.locator('.el-table__row')).toHaveCount(1);
  await expect(sourceCard).toContainText('生产交付流水线');
  await expect(sourceCard).toContainText('云效流水线');
  await expect(sourceCard).toContainText('流水线 #1001');
  await expect(sourceCard).toContainText('v1.2.0');

  await sourceCard.getByRole('button', { name: '查看制品' }).click();

  const overview = page.locator('.source-overview');
  await expect(overview.locator('.overview-title-line')).toContainText('生产交付流水线');
  await expect(overview.locator('.overview-meta')).toContainText('制品名：app');

  const stats = overview.locator('.overview-stat');
  await expect(stats).toHaveCount(3);
  await expect(stats.nth(0)).toContainText('最新版本');
  await expect(stats.nth(0)).toContainText('v1.2.0');
  await expect(stats.nth(1)).toContainText('最近构建');
  await expect(stats.nth(2)).toContainText('已加载制品');
  await expect(stats.nth(2)).toContainText('2');

  const versions = page.locator('.versions-card');
  await expect(versions.locator('.el-table__row')).toHaveCount(2);
  const latestRow = versions.locator('.el-table__row').first();
  await expect(latestRow).toContainText('v1.2.0');
  await expect(latestRow).toContainText('最新');
  await expect(latestRow).toContainText('运行 #2001');
  await expect(latestRow).toContainText('分支 main');
  await expect(latestRow).toContainText('1.0 MB');
  await expect(latestRow).toContainText('重新下载');
  const olderRow = versions.locator('.el-table__row').nth(1);
  await expect(olderRow).toContainText('v1.1.0');
  await expect(olderRow).toContainText('512.0 KB');
  await expect(olderRow).toContainText('下载');
  await expect(versions.locator('.list-footer')).toContainText('已显示全部可加载版本');

  await page.getByRole('button', { name: '返回制品源列表' }).click();
  await expect(page.locator('.source-card')).toBeVisible();
  await expect(page.getByRole('heading', { name: '选择制品源' })).toBeVisible();

  expect(problems).toEqual([]);
});

const detailPlan = {
  bundleDir: '/opt/app',
  version: 'v1.2.0',
  commitShort: 'abc1234',
  branch: 'main',
  runtime: 'compose',
  compose: 'deploy/docker-compose.yml',
  fileCount: 12,
  totalBytes: 2048,
  components: [{ key: 'web', label: 'Web 控制台', services: ['web'], artifacts: [] }],
  services: [],
  database: null,
  images: [],
  steps: [],
  imageCheck: 'local'
};

const detailConfig = {
  kind: 'compose',
  file: 'deploy/docker-compose.yml',
  sizeBytes: 512,
  imageDir: '/opt/app/bundle',
  imageDirExists: true,
  imageCheck: 'local',
  content: 'services:\n  web:\n    image: demo/web:1.2.0\n',
  services: [
    {
      name: 'web',
      image: 'demo/web:1.2.0',
      containerName: 'demo-web-1',
      ports: ['8080:80'],
      dependsOn: [],
      restart: 'unless-stopped'
    }
  ],
  images: [
    {
      imageRef: 'demo/web:1.2.0',
      status: 'ready',
      services: ['web'],
      localFile: 'web.tar',
      localBytes: 1024,
      matchSource: 'images.json',
      dockerPresent: true
    },
    {
      imageRef: 'demo/api:1.2.0',
      status: 'missing',
      services: ['web'],
      localFile: '',
      localBytes: 0,
      matchSource: '',
      dockerPresent: false
    }
  ],
  issues: []
};

test('版本详情的部署配置对话框展示镜像、服务与配置内容', async ({ page }) => {
  await useDesktopBackend(page, {
    artifactSources: [flowSource],
    artifactReleases: releases,
    bundlePlan: detailPlan,
    bundleConfig: detailConfig
  });
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '制品中心' }).click();
  await page.locator('.source-card').getByRole('button', { name: '查看制品' }).click();

  const versions = page.locator('.versions-card');
  await versions.locator('.el-table__row').first().getByRole('button', { name: '详情', exact: true }).click();
  await page.getByRole('button', { name: '查看部署配置' }).click();

  const dialog = page.locator('.compose-dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('.meta-row')).toContainText('deploy/docker-compose.yml');
  await expect(dialog.locator('.summary-card', { hasText: '就绪' })).toContainText('1');
  await expect(dialog.locator('.summary-card', { hasText: '缺失' })).toContainText('1');

  const imageRows = dialog.locator('.el-tab-pane').first().locator('.el-table__row');
  await expect(imageRows).toHaveCount(2);
  await expect(imageRows.first()).toContainText('demo/web:1.2.0');
  await expect(imageRows.first()).toContainText('已加载');
  await expect(imageRows.nth(1)).toContainText('缺失');

  await dialog.getByRole('tab', { name: '服务清单' }).click();
  await expect(dialog.locator('.el-tab-pane').nth(1).locator('.el-table__row')).toContainText('demo-web-1');

  await dialog.getByRole('tab', { name: '配置内容' }).click();
  await expect(dialog.locator('.code-view').first()).toContainText('image: demo/web:1.2.0');

  expect(problems).toEqual([]);
});
