import { expect, test } from '@playwright/test';
import { useDesktopBackend } from './support/desktop.js';

const localHost = {
  id: 'host-local',
  kind: 'local',
  name: '本机',
  address: '',
  port: 22,
  credentialId: '',
  notes: '自动化测试数据',
  updatedAt: 1_760_000_000_000
};

const localProject = {
  id: 'project-local',
  name: '演示项目',
  hostId: 'host-local',
  installDir: '/srv/deploy-demo',
  lastBundleDir: ''
};

const localEnv = {
  os: 'linux',
  arch: 'x86_64',
  os_release: 'Ubuntu 24.04',
  cpu_cores: 8,
  total_memory_mb: 16384,
  available_memory_mb: 8192,
  disk_available_gb: 120.5,
  docker_installed: true,
  docker_version: '27.1.1',
  docker_running: true,
  compose_installed: true,
  compose_version: '2.29.1',
  ports: [
    { port: 5173, protocol: 'TCP', service: '管理控制台', available: true, occupying_process: null }
  ],
  is_ready: true,
  issues: []
};

async function openWizard(page) {
  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '部署向导' }).click();
}

test('部署向导展示完整步骤条并标记当前步骤', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await openWizard(page);

  const steps = page.locator('.steps-header .stepper .step');
  await expect(steps).toHaveCount(6);
  await expect(steps).toHaveText([
    '1选择项目',
    '2检测环境',
    '3选择制品',
    '4解压制品',
    '5清单比对',
    '6选择更新'
  ]);
  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('1选择项目');
  await expect(page.locator('.steps-header')).toBeVisible();

  const empty = page.locator('.project-empty');
  await expect(empty).toContainText('当前没有可用项目');
  await expect(empty.getByRole('button', { name: '前往项目管理' })).toBeVisible();
  await expect(page.getByRole('button', { name: '下一步：检测环境' })).toBeDisabled();

  expect(problems).toEqual([]);
});

test('部署目标与环境检测步骤展示项目、主机与环境指标', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await useDesktopBackend(page, {
    hosts: [localHost],
    projects: [localProject],
    activeProjectId: localProject.id,
    localEnv
  });
  await openWizard(page);

  const target = page.locator('.step-card').first();
  await expect(target.locator('.project-select')).toContainText('演示项目');
  await expect(target).toContainText('本机');
  await expect(target.locator('.path-text')).toHaveText('/srv/deploy-demo');
  await expect(target.locator('.step-footer .field-hint')).toHaveText('部署目标：本机');

  await target.getByRole('button', { name: '下一步：检测环境' }).click();

  const environment = page.locator('.step-card').first();
  await expect(environment.locator('h3')).toHaveText('检测目标环境');
  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('2检测环境');
  await expect(environment.locator('.metric-card')).toHaveText([
    '操作系统 / 架构Ubuntu 24.04 / x86_64',
    'CPU 核心8 核',
    '物理内存16384 MB',
    '可用磁盘120.5 GB'
  ]);
  await expect(environment.locator('.status-row')).toContainText('Docker 正常运行');
  await expect(environment.locator('.status-row')).toContainText('Compose 已安装');
  await expect(environment.locator('.el-table__row')).toHaveCount(1);
  await expect(environment.locator('.el-table__row')).toContainText('管理控制台');
  await expect(environment.getByRole('button', { name: '下一步：选择制品' })).toBeEnabled();

  await environment.getByRole('button', { name: '上一步' }).click();
  await expect(page.locator('.step-card').first().locator('h3')).toHaveText('确认部署目标');
  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('1选择项目');

  expect(problems).toEqual([]);
});
