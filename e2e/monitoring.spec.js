import { expect, test } from '@playwright/test';
import { remoteHost, useDesktopBackend } from './support/desktop.js';

const project = {
  id: 'project-monitor',
  name: '测试项目',
  hostId: remoteHost.id,
  installDir: '/opt/demo',
  sourceId: ''
};

const bundle = {
  dir: '/opt/bundles/demo',
  version: '1.2.0',
  containerPrefix: 'demo',
  services: [
    { key: 'web', label: 'Web 服务', port: 8080, protocol: 'TCP', description: '对外网页入口' },
    { key: 'api', label: '接口服务', port: 9000 }
  ]
};

const containers = [
  {
    name: 'demo-web',
    service_key: 'web',
    state: 'running',
    health: 'healthy',
    status_text: '运行 2 小时',
    ports: '0.0.0.0:8080->80/tcp',
    image: 'demo/web:1.2.0'
  },
  {
    name: 'demo-api',
    service_key: 'api',
    state: 'exited',
    health: '',
    status_text: '',
    ports: '',
    image: 'demo/api:1.2.0'
  }
];

test('监控页展示服务端口、集群健康与服务实例', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await useDesktopBackend(page, {
    hosts: [remoteHost],
    projects: [project],
    activeProjectId: project.id,
    bundle,
    containers
  });

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '项目管理' }).click();
  await page.getByRole('button', { name: '监控 测试项目' }).click();

  await expect(page.locator('.page-title')).toHaveText('集群服务监控');

  const endpoints = page.locator('.endpoint-grid .endpoint-card');
  await expect(endpoints).toHaveCount(3);
  await expect(endpoints.first()).toContainText('Web 服务');
  await expect(endpoints.first()).toContainText('192.0.2.10:8080');
  await expect(endpoints.nth(1)).toContainText('端口 9000 · TCP');

  const health = page.locator('.health-card');
  await expect(health).toContainText('需关注');
  await expect(health).toContainText('1 / 2');
  await expect(health).toContainText('存在未运行或健康检查异常的服务');

  await expect(page.locator('.section-head')).toContainText('1 个运行中，共 2 个服务');
  const cards = page.locator('.containers-grid .container-card');
  await expect(cards).toHaveCount(2);
  await expect(cards.first()).toContainText('HEALTHY');
  await expect(cards.first()).toContainText('对外网页入口');
  await expect(cards.first()).toContainText('demo/web:1.2.0');
  await expect(cards.nth(1)).toContainText('EXITED');
  await expect(cards.nth(1)).toContainText('未运行');

  await cards.first().getByRole('button', { name: '查看日志' }).click();
  await expect(page.locator('.page-title')).toHaveText('容器日志诊断');

  expect(problems).toEqual([]);
});

test('监控页在没有运行实例时展示空状态', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await useDesktopBackend(page, {
    hosts: [remoteHost],
    projects: [project],
    activeProjectId: project.id,
    bundle,
    containers: []
  });

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '项目管理' }).click();
  await page.getByRole('button', { name: '监控 测试项目' }).click();

  await expect(page.locator('.health-card')).toContainText('未知');
  await expect(page.locator('.health-card')).toContainText('等待读取容器状态');
  await expect(page.locator('.cluster-empty-title')).toHaveText('未读取到服务实例');

  expect(problems).toEqual([]);
});
