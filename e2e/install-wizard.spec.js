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

const demoPreparedBundle = {
  dir: '/tmp/bundle-demo',
  version: 'v1.4.0',
  commit: 'abc1234',
  md5: '',
  sizeBytes: 2048,
  files: 12
};

const demoBundlePlan = {
  bundleDir: '/tmp/bundle-demo',
  product: 'demo',
  displayName: '演示产品',
  version: 'v1.4.0',
  buildNumber: '104',
  commit: 'abc1234',
  commitShort: 'abc1234',
  branch: 'main',
  builtAt: '2026-10-01T08:00:00Z',
  runtime: 'docker',
  compose: 'deploy/docker-compose.yml',
  variant: 'standard',
  components: [
    { key: 'web', label: 'Web 控制台', services: ['web'], artifacts: [] },
    { key: 'api', label: '接口服务', services: ['api'], artifacts: [] }
  ],
  services: [],
  database: null,
  images: [
    {
      imageRef: 'demo/web:1.4.0',
      role: 'app',
      required: true,
      bundleFile: 'images/web.tar',
      source: 'compose',
      presentLocally: true
    },
    {
      imageRef: 'demo/api:1.4.0',
      role: 'app',
      required: true,
      bundleFile: '',
      source: 'compose',
      presentLocally: false
    }
  ],
  imageCheck: 'checked',
  fileCount: 12,
  totalBytes: 2048,
  steps: []
};

const demoDeploymentReview = {
  composeFile: 'deploy/docker-compose.yml',
  targetComposeFile: '/srv/deploy-demo/docker-compose.yml',
  currentCompose: 'services:\n  web:\n    image: demo/web:1.3.0\n',
  desiredCompose: 'services:\n  web:\n    image: demo/web:1.4.0\n  api:\n    image: demo/api:1.4.0\n',
  installDir: '/srv/deploy-demo',
  hasDeployment: true,
  targetChecked: true,
  imageCheck: 'checked',
  dockerRunning: true,
  composeAvailable: true,
  composeChanged: true,
  services: [
    {
      name: 'web',
      image: 'demo/web:1.4.0',
      currentImage: 'demo/web:1.3.0',
      containerName: 'demo-web-1',
      containerState: 'running',
      containerHealth: 'healthy',
      ports: ['8080:80'],
      dependsOn: [],
      profiles: [],
      change: 'changed'
    },
    {
      name: 'api',
      image: 'demo/api:1.4.0',
      currentImage: '',
      containerName: '',
      containerState: 'not_created',
      containerHealth: 'none',
      ports: [],
      dependsOn: ['web'],
      profiles: [],
      change: 'added'
    }
  ],
  images: [
    {
      imageRef: 'demo/web:1.4.0',
      services: ['web'],
      dockerPresent: true,
      status: 'ready',
      localFile: 'images/web.tar'
    },
    {
      imageRef: 'demo/api:1.4.0',
      services: ['api'],
      dockerPresent: false,
      status: 'missing',
      localFile: ''
    }
  ],
  missingImages: ['demo/api:1.4.0'],
  issues: [],
  ready: true,
  canFirstDeploy: false,
  canIncrementalUpdate: true
};

async function openReviewStep(page) {
  await openWizard(page);
  await page.getByRole('button', { name: '下一步：检测环境' }).click();
  await page.getByRole('button', { name: '下一步：选择制品' }).click();
  await page.getByRole('button', { name: '选择已解压目录' }).click();
  await expect(page.locator('.selection-summary')).toContainText('已选择：bundle-demo');
  await page.getByRole('button', { name: '下一步：解压制品' }).click();
  await page.getByRole('button', { name: '校验交付包' }).click();
  await expect(page.locator('.prepare-panel .el-alert')).toContainText('交付包已准备就绪');
  await page.getByRole('button', { name: '下一步：清单比对' }).click();
}

test('清单比对步骤展示服务、镜像与配置差异', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await useDesktopBackend(page, {
    hosts: [localHost],
    projects: [localProject],
    activeProjectId: localProject.id,
    localEnv,
    dialogPath: demoPreparedBundle.dir,
    preparedBundle: demoPreparedBundle,
    bundlePlan: demoBundlePlan,
    deploymentReview: demoDeploymentReview
  });

  await openReviewStep(page);

  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('5清单比对');
  const review = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '核对服务清单与镜像' }) });
  await expect(review).toHaveCount(1);
  await expect(review.locator('.status-row')).toContainText('检测到现有部署');
  await expect(review.locator('.status-row')).toContainText('制品版本 v1.4.0');
  await expect(review.locator('.status-row')).toContainText('2 个服务');
  await expect(review.locator('.compose-review-heading')).toContainText(
    '部署清单：deploy/docker-compose.yml'
  );
  await expect(review.getByText('配置有变化，可继续选择更新内容', { exact: false })).toBeVisible();

  // 只统计当前标签页的表格行，未激活的标签页仍留在 DOM 中。
  const activeRows = () => review.getByRole('tabpanel').locator('.el-table__row');
  await expect(activeRows()).toHaveCount(2);
  await expect(activeRows().nth(0)).toContainText('web');
  await expect(activeRows().nth(0)).toContainText('demo/web:1.4.0');
  await expect(activeRows().nth(0)).toContainText('demo/web:1.3.0');
  await expect(activeRows().nth(0)).toContainText('有变化');
  await expect(activeRows().nth(0)).toContainText('运行中');
  await expect(activeRows().nth(1)).toContainText('新增');
  await expect(activeRows().nth(1)).toContainText('未创建');

  await review.getByRole('tab', { name: '镜像核对' }).click();
  await expect(activeRows().nth(0)).toContainText('demo/web:1.4.0');
  await expect(activeRows().nth(0)).toContainText('已具备');
  await expect(activeRows().nth(1)).toContainText('demo/api:1.4.0');
  await expect(activeRows().nth(1)).toContainText('缺失');

  await expect(review.locator('.image-preparation')).toContainText('准备缺失镜像');
  await expect(review.getByRole('button', { name: '导入镜像并比对' })).toBeVisible();
  await expect(review.getByRole('button', { name: '下一步：选择更新' })).toBeEnabled();

  await review.getByRole('button', { name: '查看配置对比' }).click();
  const dialog = page.locator('.config-diff-dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('部署配置对比');
  await expect(dialog.locator('.diff-summary')).toContainText('差异');
  await dialog.getByRole('button', { name: '关闭' }).click();
  await expect(dialog).toBeHidden();

  await review.getByRole('button', { name: '上一步' }).click();
  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('4解压制品');
  await expect(page.locator('.step-card').first().locator('h3')).toHaveText('解压并校验制品');

  expect(problems).toEqual([]);
});

test('清单比对步骤在没有镜像缺失时不显示准备面板', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await useDesktopBackend(page, {
    hosts: [localHost],
    projects: [localProject],
    activeProjectId: localProject.id,
    localEnv,
    dialogPath: demoPreparedBundle.dir,
    preparedBundle: demoPreparedBundle,
    bundlePlan: demoBundlePlan,
    deploymentReview: {
      ...demoDeploymentReview,
      composeChanged: false,
      missingImages: [],
      images: demoDeploymentReview.images.map((item) => ({ ...item, status: 'ready' })),
      services: demoDeploymentReview.services.map((item) => ({
        ...item,
        change: 'unchanged',
        currentImage: item.image
      }))
    }
  });

  await openReviewStep(page);

  const review = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '核对服务清单与镜像' }) });
  await expect(review.locator('.status-row')).toContainText('2 个服务');
  await expect(review.getByText('配置有变化，可继续选择更新内容', { exact: false })).toBeHidden();
  await expect(review.locator('.image-preparation')).toHaveCount(0);
  const serviceRows = review.getByRole('tabpanel').locator('.el-table__row');
  await expect(serviceRows).toHaveCount(2);
  await expect(serviceRows.nth(0)).toContainText('一致');
  await expect(serviceRows.nth(1)).toContainText('一致');

  expect(problems).toEqual([]);
});

test('选择更新步骤展示部署方式与更新内容', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await useDesktopBackend(page, {
    hosts: [localHost],
    projects: [localProject],
    activeProjectId: localProject.id,
    localEnv,
    dialogPath: demoPreparedBundle.dir,
    preparedBundle: demoPreparedBundle,
    bundlePlan: demoBundlePlan,
    deploymentReview: demoDeploymentReview
  });

  await openReviewStep(page);

  const review = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '核对服务清单与镜像' }) });
  await review.getByRole('button', { name: '下一步：选择更新' }).click();

  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('6选择更新');
  const step = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '选择部署操作' }) });
  await expect(step.locator('.desc')).toContainText('部署目录 /srv/deploy-demo');
  await expect(step.locator('.desc')).toContainText('制品版本 v1.4.0');

  const actions = step.locator('.deployment-actions .el-radio');
  await expect(actions).toHaveCount(2);
  await expect(actions.nth(0)).toContainText('首次部署');
  await expect(actions.nth(0)).toHaveClass(/is-disabled/);
  await expect(actions.nth(1)).toContainText('增量更新');
  await expect(actions.nth(1)).toHaveClass(/is-checked/);

  const plan = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '选择部署操作' }) })
    .locator('form');
  await expect(plan.getByText('更新内容')).toBeVisible();
  // 第三个选项来自交付包随附的镜像包。
  const checkboxes = plan.locator('.el-checkbox');
  await expect(checkboxes).toHaveCount(3);
  await expect(checkboxes.nth(0)).toContainText('Web 控制台');
  await expect(checkboxes.nth(1)).toContainText('接口服务');
  await expect(checkboxes.nth(2)).toContainText('随包镜像');
  await expect(plan.getByText('更新前备份')).toBeVisible();

  await expect(step.getByRole('button', { name: '执行预检' })).toBeEnabled();
  await expect(step.getByRole('button', { name: '开始增量更新' })).toBeEnabled();
  await expect(step.getByRole('button', { name: '查看集群监控' })).toHaveCount(0);

  await checkboxes.nth(0).click();
  await checkboxes.nth(1).click();
  await checkboxes.nth(2).click();
  await expect(step.getByRole('button', { name: '开始增量更新' })).toBeDisabled();

  await checkboxes.nth(0).click();
  await expect(step.getByRole('button', { name: '开始增量更新' })).toBeEnabled();

  await step.getByRole('button', { name: '返回清单比对' }).click();
  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('5清单比对');

  expect(problems).toEqual([]);
});
