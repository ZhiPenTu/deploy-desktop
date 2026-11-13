import { expect, test } from '@playwright/test';
import { remoteHost, sshCredential, useDesktopBackend } from './support/desktop.js';

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

const offlineEnv = {
  ...localEnv,
  docker_running: false,
  compose_installed: false,
  is_ready: false,
  issues: ['Docker 服务未处于运行状态']
};

const dockerOfflineCatalog = {
  packages: [
    {
      platform: 'linux',
      arch: 'x86_64',
      version: '29.9.3',
      build: '',
      fileName: 'docker-29.9.3.tgz'
    },
    {
      platform: 'windows',
      arch: 'amd64',
      version: '4.98.0',
      build: '245422',
      fileName: 'Docker-Desktop-4.98.0-245422-amd64.exe'
    },
    {
      platform: 'windows',
      arch: 'amd64',
      version: '4.97.0',
      build: '244584',
      fileName: 'Docker-Desktop-4.97.0-244584-amd64.exe'
    },
    {
      platform: 'macos',
      arch: 'arm64',
      version: '4.98.0',
      build: '245422',
      fileName: 'Docker-4.98.0-245422-arm64.dmg'
    }
  ],
  defaultPlatform: 'windows:amd64',
  downloadDir: '/tmp/downloads',
  warnings: []
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
    '6部署版本'
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

test('离线 Docker 引擎可下载对应平台的安装包', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await useDesktopBackend(page, {
    hosts: [localHost],
    projects: [localProject],
    activeProjectId: localProject.id,
    localEnv: offlineEnv,
    dialogPath: '/tmp/downloads/Docker-Desktop-4.97.0-244584-amd64.exe',
    dockerOfflineCatalog
  });
  await openWizard(page);
  await page.getByRole('button', { name: '下一步：检测环境' }).click();

  const environment = page.locator('.step-card').first();
  await expect(environment.getByText('安装离线 Docker 引擎')).toBeVisible();
  await environment.getByRole('button', { name: '下载离线包', exact: true }).click();

  const dialog = page.getByRole('dialog', { name: '下载 Docker 离线包' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('Windows（x64）');
  await expect(dialog).toContainText('4.98.0（构建 245422）');
  await expect(dialog).toContainText('Docker-Desktop-4.98.0-245422-amd64.exe');

  await dialog.locator('.el-select').first().click();
  await page.getByRole('option', { name: 'Linux（x86_64）' }).click();
  await expect(dialog).toContainText('docker-29.9.3.tgz');
  await expect(dialog).toContainText('docker-engine 子目录');

  await dialog.locator('.el-select').first().click();
  await page.getByRole('option', { name: 'Windows（x64）' }).click();
  await dialog.locator('.el-select').nth(1).click();
  await page.getByRole('option', { name: '4.97.0（构建 244584）' }).click();
  await expect(dialog).toContainText('Docker-Desktop-4.97.0-244584-amd64.exe');

  await dialog.getByRole('button', { name: '下载', exact: true }).click();
  await expect(dialog.getByText('离线包已下载')).toBeVisible();
  await expect(dialog).toContainText('/tmp/downloads/Docker-Desktop-4.97.0-244584-amd64.exe');

  const call = await page.evaluate(() =>
    window.__deployTestCalls.find((item) => item.command === 'download_docker_offline_package')
  );
  expect(call.args).toMatchObject({
    platform: 'windows',
    arch: 'amd64',
    version: '4.97.0',
    build: '244584'
  });
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
  services: [
    { key: 'web', label: 'Web 控制台', role: 'app' },
    { key: 'api', label: '接口服务', role: 'app' }
  ],
  dataVolumes: [{ name: 'demo-data', service: 'web', image: 'nginx', required: true }],
  dataPaths: [{ path: 'deploy/backups', mode: 'preserve' }],
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
  targetPlatform: 'linux/amd64',
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
      platform: 'linux/amd64',
      targetPlatform: 'linux/amd64',
      detail: '目标 Docker 已具备该镜像',
      pullable: false,
      localFile: 'images/web.tar'
    },
    {
      imageRef: 'demo/api:1.4.0',
      services: ['api'],
      dockerPresent: false,
      status: 'missing',
      platform: '',
      targetPlatform: 'linux/amd64',
      detail: '目标 Docker 中没有该镜像，可拉取或导入',
      pullable: true,
      localFile: ''
    }
  ],
  artifacts: [
    {
      key: 'compose',
      label: '编排文件',
      source: 'deploy/docker-compose.yml',
      root: 'deploy',
      path: 'docker-compose.yml',
      target: '/srv/deploy-demo/deploy/docker-compose.yml',
      kind: 'file',
      status: 'present',
      expectedBytes: 128,
      actualBytes: 128,
      expectedFiles: 1,
      actualFiles: 1
    }
  ],
  artifactIssues: [],
  missingImages: ['demo/api:1.4.0'],
  unreadyImages: ['demo/api:1.4.0'],
  issues: [],
  ready: false,
  canFirstDeploy: false,
  canIncrementalUpdate: false
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

async function setupImageTransfer(page, options = {}) {
  const project = { ...localProject, id: 'remote-image-project', hostId: remoteHost.id };
  const reference = options.digest ? `demo/api@sha256:${'a'.repeat(64)}` : 'demo/api:1.4.0';
  const initialReview = {
    ...demoDeploymentReview,
    ready: false,
    canIncrementalUpdate: false,
    missingImages: [reference],
    unreadyImages: [reference],
    images: demoDeploymentReview.images.map((image) =>
      image.imageRef === 'demo/api:1.4.0' ? { ...image, imageRef: reference } : image)
  };
  await useDesktopBackend(page, {
    hosts: [remoteHost], credentials: [sshCredential], projects: [project],
    activeProjectId: project.id, remoteEnv: localEnv,
    dialogPath: demoPreparedBundle.dir, preparedBundle: demoPreparedBundle,
    bundlePlan: demoBundlePlan,
    deploymentReview: initialReview,
    registrySources: options.registrySources || []
  });
  await page.addInitScript(({ initialReview, options, reference }) => {
    const original = window.__TAURI_INTERNALS__.invoke;
    let imported = false;
    let pulled = false;
    let bound = false;
    window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      const handled = ['export_deployment_images', 'prepare_local_deployment_images',
        'upload_remote_image_bundle', 'import_remote_image_bundle', 'scan_local_image_bundle',
        'pull_deployment_images', 'register_deployment_image_bindings'];
      if (handled.includes(command)) {
        window.__deployTestCalls.push({ command, args });
        if (command === options.failCommand) throw '测试操作失败，请重新比对。';
        if ((command === 'export_deployment_images' || command === 'prepare_local_deployment_images')
            && options.pauseExport) {
          await new Promise((resolve) => { window.__finishImageExport = resolve; });
        }
        if (command === 'export_deployment_images' || command === 'prepare_local_deployment_images') return [{
          reference, fileName: 'api.tar', path: '/offline/api.tar',
          binding: options.digest ? {
            reference, imageId: `sha256:${'b'.repeat(64)}`, platform: 'linux/amd64'
          } : null,
          success: !options.exportFailed, message: '本机缺少镜像，请选择离线镜像包'
        }];
        if (command === 'scan_local_image_bundle') return { files: [{ name: 'api.tar.gz' }, { name: 'other.tar' }] };
        if (command === 'import_remote_image_bundle') imported = true;
        if (command === 'register_deployment_image_bindings') bound = true;
        if (command === 'pull_deployment_images') {
          pulled = true;
          return [{ reference, message: '已在目标主机拉取并确认' }];
        }
        return ['完成'];
      }
      if (command === 'inspect_deployment_review' && (imported || pulled || options.offline)) {
        window.__deployTestCalls.push({ command, args });
        const ready = (pulled || (imported && (!options.digest || bound))) && !options.stillMissing;
        return {
          ...initialReview, ready, canIncrementalUpdate: ready,
          missingImages: ready ? [] : [reference],
          unreadyImages: ready ? [] : [reference],
          images: initialReview.images.map((image) => image.imageRef === reference && ready
            ? {
                ...image,
                dockerPresent: true,
                status: 'ready',
                platform: 'linux/amd64',
                detail: '目标 Docker 已具备该镜像',
                pullable: false,
                localFile: 'api.tar.gz'
              }
            : { ...image,
                localFile: image.imageRef === reference ? 'api.tar.gz' : image.localFile
              })
        };
      }
      return original(command, args);
    };
  }, { initialReview, options, reference });
  await openReviewStep(page);
  await page.getByRole('tab', { name: '镜像与修复' }).click();
  await page.locator('.image-preparation')
    .getByText(options.pull ? '从镜像仓库拉取' : '本机拉取后传入', { exact: true })
    .click();
}

async function setupLocalImagePull(page, options = {}) {
  const initialReview = options.platformMismatch
    ? {
        ...demoDeploymentReview,
        targetPlatform: 'linux/arm64',
        missingImages: [],
        unreadyImages: ['demo/api:1.4.0'],
        images: demoDeploymentReview.images.map((image) =>
          image.imageRef === 'demo/api:1.4.0'
            ? {
                ...image,
                dockerPresent: true,
                status: 'platform_mismatch',
                platform: 'linux/amd64',
                targetPlatform: 'linux/arm64',
                detail: '现有镜像为 linux/amd64，目标为 linux/arm64；可重新拉取适配架构的版本',
                pullable: true
              }
            : image
        )
      }
    : { ...demoDeploymentReview, ready: false, canIncrementalUpdate: false };
  await useDesktopBackend(page, {
    hosts: [localHost],
    projects: [localProject],
    activeProjectId: localProject.id,
    localEnv,
    dialogPath: demoPreparedBundle.dir,
    preparedBundle: demoPreparedBundle,
    bundlePlan: demoBundlePlan,
    deploymentReview: initialReview,
    registrySources: options.registrySources || []
  });
  await page.addInitScript(({ initialReview, options }) => {
    const original = window.__TAURI_INTERNALS__.invoke;
    let pulled = false;
    window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      if (command === 'pull_deployment_images') {
        window.__deployTestCalls.push({ command, args });
        if (options.failPull) throw '拉取镜像失败，请检查网络和仓库权限。';
        pulled = true;
        return [{ reference: 'demo/api:1.4.0', message: '已拉取并确认' }];
      }
      if (command === 'inspect_deployment_review' && pulled) {
        window.__deployTestCalls.push({ command, args });
        const ready = !options.stillMissingAfterPull;
        return {
          ...initialReview,
          ready,
          canIncrementalUpdate: ready,
          missingImages: ready ? [] : initialReview.missingImages,
          unreadyImages: ready ? [] : initialReview.unreadyImages,
          images: initialReview.images.map((image) => image.imageRef === 'demo/api:1.4.0' && ready
            ? {
                ...image,
                dockerPresent: true,
                status: 'ready',
                platform: 'linux/amd64',
                detail: '目标 Docker 已具备该镜像',
                pullable: false
              }
            : image)
        };
      }
      return original(command, args);
    };
  }, { initialReview, options });
  await openReviewStep(page);
  await page.getByRole('tab', { name: '镜像与修复' }).click();
}

test('局域网目标可从本机拉取并传入镜像，核对通过后才能继续', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => { if (['error', 'warning'].includes(message.type())) problems.push(message.text()); });
  await page.setViewportSize({ width: 1440, height: 1080 });
  await setupImageTransfer(page);
  await expect(page).toHaveTitle(/部署/);
  expect(page.url()).toContain('127.0.0.1:4173');
  await expect(page.locator('vite-error-overlay')).toHaveCount(0);
  await expect(page.getByRole('radio', { name: '本机拉取后传入', exact: true })).toBeChecked();
  await expect(page.getByRole('button', { name: '下一步：部署版本' })).toBeDisabled();
  await page.screenshot({ path: '/tmp/deploy-missing-images-desktop.png', fullPage: true, animations: 'disabled' });
  await page.getByRole('button', { name: '本机拉取并传入' }).click();
  const dialog = page.getByRole('dialog', { name: '确认本机拉取并传入' });
  await expect(dialog).toContainText(remoteHost.name);
  await expect(dialog).toContainText('1 个可处理镜像');
  await expect(dialog).toContainText('目标机无需访问镜像仓库');
  await expect(dialog).toContainText('运行中的服务不会重启');
  await expect(
    page.locator('.image-preparation').getByRole('button', { name: '重新检查', exact: true })
  ).toBeDisabled();
  await dialog.getByRole('button', { name: '拉取并传入', exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(
    page.getByText('本机镜像已按目标架构传入并导入，目标镜像核对通过。', { exact: true })
  ).toBeVisible();
  await expect(page.getByRole('button', { name: '下一步：部署版本' })).toBeEnabled();
  await expect(page.locator('.image-preparation')).toContainText('镜像已就绪');
  const calls = await page.evaluate(() => window.__deployTestCalls);
  const writes = calls.filter((item) =>
    ['prepare_local_deployment_images', 'upload_remote_image_bundle', 'import_remote_image_bundle'].includes(item.command));
  expect(writes.map((item) => item.command)).toEqual([
    'prepare_local_deployment_images', 'upload_remote_image_bundle', 'import_remote_image_bundle'
  ]);
  expect(writes[0].args.references).toEqual(['demo/api:1.4.0']);
  expect(writes[1].args).toMatchObject({ host: remoteHost.address, port: remoteHost.port,
    user: sshCredential.username, localDir: '/offline', files: ['api.tar'], overwrite: false });
  expect(writes[2].args.remoteBundleDir).toBe(writes[1].args.remoteBundleDir);
  await page.screenshot({ path: '/tmp/deploy-missing-images-complete.png', fullPage: true, animations: 'disabled' });
  expect(problems).toEqual([]);
});

test('摘要镜像传入后核对内容再登记，通过后才允许继续', async ({ page }) => {
  await setupImageTransfer(page, { digest: true });
  await expect(page.getByRole('button', { name: '本机拉取并传入' })).toBeEnabled();
  await page.getByRole('button', { name: '本机拉取并传入' }).click();
  await page.getByRole('dialog', { name: '确认本机拉取并传入' })
    .getByRole('button', { name: '拉取并传入', exact: true }).click();
  await expect(page.getByText('本机镜像已按目标架构传入并导入，目标镜像核对通过。', { exact: true })).toBeVisible();
  const calls = await page.evaluate(() => window.__deployTestCalls);
  const registration = calls.find((call) => call.command === 'register_deployment_image_bindings');
  expect(registration.args.references).toEqual([`demo/api@sha256:${'a'.repeat(64)}`]);
  expect(registration.args.expected).toEqual([{
    reference: `demo/api@sha256:${'a'.repeat(64)}`,
    imageId: `sha256:${'b'.repeat(64)}`,
    platform: 'linux/amd64'
  }]);
  await expect(page.getByRole('button', { name: '下一步：部署版本' })).toBeEnabled();
});

test('摘要镜像内容核对失败时不显示传入成功', async ({ page }) => {
  await setupImageTransfer(page, { digest: true, failCommand: 'register_deployment_image_bindings' });
  await page.getByRole('button', { name: '本机拉取并传入' }).click();
  await page.getByRole('dialog', { name: '确认本机拉取并传入' })
    .getByRole('button', { name: '拉取并传入', exact: true }).click();
  await expect(page.locator('.step-card .el-alert--error')).toBeVisible();
  await expect(page.getByText('本机镜像已按目标架构传入并导入，目标镜像核对通过。', { exact: true })).toBeHidden();
  await expect(page.getByRole('button', { name: '下一步：部署版本' })).toBeDisabled();
});

test('本机可按清单从镜像仓库拉取待处理镜像并重新核对', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => { if (['error', 'warning'].includes(message.type())) problems.push(message.text()); });
  await setupLocalImagePull(page);

  const preparation = page.locator('.image-preparation');
  await expect(page.getByRole('radio', { name: '从镜像仓库拉取', exact: true })).toBeChecked();
  await expect(preparation).toContainText('缺失 1 个');
  await expect(preparation).toContainText('未找到匹配的镜像源');
  await expect(preparation.getByRole('button', { name: '拉取 1 个镜像' })).toBeVisible();
  await page.screenshot({ path: '/tmp/deploy-image-pull-local.png', fullPage: true, animations: 'disabled' });

  await preparation.getByRole('button', { name: '拉取 1 个镜像' }).click();
  const dialog = page.getByRole('dialog', { name: '确认拉取镜像' });
  await expect(dialog).toContainText('在本机按清单地址拉取 1 个可处理镜像到本机 Docker');
  await expect(dialog).toContainText('运行中的服务不会重启');
  await dialog.getByRole('button', { name: '开始拉取', exact: true }).click();

  await expect(page.getByText('待处理镜像已拉取，目标镜像核对通过。', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: '下一步：部署版本' })).toBeEnabled();
  const calls = await page.evaluate(() => window.__deployTestCalls);
  const pull = calls.find((item) => item.command === 'pull_deployment_images');
  expect(pull.args).toMatchObject({
    bundleDir: demoPreparedBundle.dir,
    references: ['demo/api:1.4.0'],
    host: null,
    port: null,
    user: null,
    keyPath: null,
    registrySourceId: null,
    password: null,
    installDir: '/srv/deploy-demo',
    registryCredentials: []
  });
  expect(problems).toEqual([]);
});

test('镜像拉取失败时保留缺失状态并允许重试', async ({ page }) => {
  await setupLocalImagePull(page, { failPull: true });
  await page.getByRole('button', { name: '拉取 1 个镜像' }).click();
  await page.getByRole('dialog', { name: '确认拉取镜像' })
    .getByRole('button', { name: '开始拉取', exact: true })
    .click();

  await expect(page.locator('.step-card .el-alert--error')).toBeVisible();
  await expect(page.locator('.step-card .el-alert--error')).toContainText('拉取镜像失败');
  await expect(page.locator('.step-card .el-alert--error')).not.toContainText('Error:');
  await expect(page.getByRole('button', { name: '下一步：部署版本' })).toBeDisabled();
  await expect(page.getByRole('button', { name: '拉取 1 个镜像' })).toBeEnabled();
  await expect(page.getByText('待处理镜像已拉取，目标镜像核对通过。', { exact: true })).toBeHidden();
  await page.getByText('查看镜像处理详情', { exact: true }).click();
  await expect(page.locator('.image-diagnostics pre')).not.toBeEmpty();
});

test('架构不匹配会说明原因并提供重新拉取按钮', async ({ page }) => {
  await setupLocalImagePull(page, { platformMismatch: true });
  const review = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '核对目标环境' }) });
  const rows = review.getByRole('tabpanel').locator('.el-table__row');
  await expect(rows.nth(1)).toContainText('架构不匹配');
  await expect(rows.nth(1)).toContainText('现有镜像为 linux/amd64，目标为 linux/arm64');
  await expect(review.getByText('未确认', { exact: true })).toHaveCount(0);
  await expect(review.getByRole('button', { name: '下一步：部署版本' })).toBeDisabled();

  await review.getByRole('button', { name: '拉取 1 个镜像' }).click();
  await page.getByRole('dialog', { name: '确认拉取镜像' })
    .getByRole('button', { name: '开始拉取', exact: true })
    .click();
  await expect(page.getByText('待处理镜像已拉取，目标镜像核对通过。', { exact: true })).toBeVisible();
  await expect(review.getByRole('button', { name: '下一步：部署版本' })).toBeEnabled();
});

test('拉取成功但复核仍缺失时不显示成功', async ({ page }) => {
  await setupLocalImagePull(page, { stillMissingAfterPull: true });
  await page.getByRole('button', { name: '拉取 1 个镜像' }).click();
  await page.getByRole('dialog', { name: '确认拉取镜像' })
    .getByRole('button', { name: '开始拉取', exact: true })
    .click();

  await expect(page.locator('.step-card .el-alert--error')).toContainText(
    '仍有 1 个镜像未就绪'
  );
  await expect(page.getByRole('button', { name: '下一步：部署版本' })).toBeDisabled();
  await expect(page.getByText('待处理镜像已拉取，目标镜像核对通过。', { exact: true })).toBeHidden();
});

test('远程拉取按仓库分组显示镜像源并使用本次凭据', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => { if (['error', 'warning'].includes(message.type())) problems.push(message.text()); });
  await setupImageTransfer(page, {
    pull: true,
    registrySources: [
      {
        id: 'registry-docker',
        name: 'Docker Hub',
        address: 'docker.io',
        namespace: '',
        username: 'reader',
        hasPassword: false
      },
      {
        id: 'registry-unrelated',
        name: '其他仓库',
        address: 'registry.example.com',
        namespace: '',
        username: 'reader',
        hasPassword: true
      }
    ]
  });

  await expect(page.getByRole('radio', { name: '从镜像仓库拉取', exact: true })).toBeChecked();
  const sourceSelect = page.locator('.registry-group .el-select');
  await sourceSelect.click();
  await expect(page.getByRole('option', { name: /Docker Hub/ })).toBeVisible();
  await expect(page.getByRole('option', { name: /其他仓库/ })).toHaveCount(0);
  await page.getByRole('option', { name: /Docker Hub/ }).click();
  await page.getByRole('textbox', { name: '镜像源密码' }).fill('secret');
  await page.screenshot({ path: '/tmp/deploy-image-pull-remote.png', fullPage: true, animations: 'disabled' });

  await page.getByRole('button', { name: '拉取 1 个镜像' }).click();
  const dialog = page.getByRole('dialog', { name: '确认拉取镜像' });
  await expect(dialog).toContainText(`到${remoteHost.name}`);
  await dialog.getByRole('button', { name: '开始拉取', exact: true }).click();

  await expect(page.getByText('待处理镜像已拉取，目标镜像核对通过。', { exact: true })).toBeVisible();
  const calls = await page.evaluate(() => window.__deployTestCalls);
  const pull = calls.find((item) => item.command === 'pull_deployment_images');
  expect(pull.args).toMatchObject({
    references: ['demo/api:1.4.0'],
    host: remoteHost.address,
    port: remoteHost.port,
    user: sshCredential.username,
    registrySourceId: null,
    password: null,
    registryCredentials: [{ registrySourceId: 'registry-docker', password: 'secret' }]
  });
  expect(problems).toEqual([]);
});

test('重新检查后保留镜像源选择并沿用本次填写的密码', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(error.message));
  page.on('console', (message) => { if (['error', 'warning'].includes(message.type())) problems.push(message.text()); });
  await setupImageTransfer(page, {
    pull: true,
    registrySources: [
      {
        id: 'registry-docker',
        name: 'Docker Hub',
        address: 'docker.io',
        namespace: '',
        username: 'reader',
        hasPassword: true
      }
    ]
  });

  const sourceSelect = page.locator('.registry-group .el-select');
  await sourceSelect.click();
  await page.getByRole('option', { name: /Docker Hub/ }).click();
  const password = page.getByRole('textbox', { name: '镜像源密码' });
  await expect(password).toHaveAttribute('placeholder', '留空使用已保存的密码');
  await password.fill('override-secret');

  await page.locator('.image-preparation')
    .getByRole('button', { name: '重新检查', exact: true })
    .click();
  await expect(page.locator('.registry-group .el-select')).toContainText('Docker Hub（docker.io）');
  await expect(page.getByRole('textbox', { name: '镜像源密码' })).toHaveValue('override-secret');

  await page.getByRole('button', { name: '拉取 1 个镜像' }).click();
  await page.getByRole('dialog', { name: '确认拉取镜像' })
    .getByRole('button', { name: '开始拉取', exact: true })
    .click();
  await expect(page.getByText('待处理镜像已拉取，目标镜像核对通过。', { exact: true })).toBeVisible();
  const calls = await page.evaluate(() => window.__deployTestCalls);
  const pull = calls.find((item) => item.command === 'pull_deployment_images');
  expect(pull.args.registryCredentials).toEqual([
    { registrySourceId: 'registry-docker', password: 'override-secret' }
  ]);
  expect(problems).toEqual([]);
});

test('按摘要固定的镜像不通过离线镜像包传输，并说明需要目标主机拉取', async ({ page }) => {
  const digestReference = `demo/api@sha256:${'a'.repeat(64)}`;
  const project = { ...localProject, id: 'digest-image-project', hostId: remoteHost.id };
  await useDesktopBackend(page, {
    hosts: [remoteHost], credentials: [sshCredential], projects: [project],
    activeProjectId: project.id, remoteEnv: localEnv,
    dialogPath: demoPreparedBundle.dir, preparedBundle: demoPreparedBundle,
    bundlePlan: demoBundlePlan,
    deploymentReview: {
      ...demoDeploymentReview,
      ready: false,
      canIncrementalUpdate: false,
      missingImages: [digestReference],
      unreadyImages: [digestReference],
      images: [
        { ...demoDeploymentReview.images[0], status: 'ready', pullable: false },
        {
          ...demoDeploymentReview.images[1],
          imageRef: digestReference,
          detail: '目标 Docker 中没有该镜像，可拉取或导入',
          pullable: true
        }
      ]
    }
  });
  await openReviewStep(page);
  await page.getByRole('tab', { name: '镜像与修复' }).click();
  await page.locator('.image-preparation').getByText('离线镜像包', { exact: true }).click();
  await expect(page.locator('.digest-alert')).toContainText('离线镜像包无法核对仓库摘要');
  await expect(page.locator('.digest-alert')).toContainText(digestReference);
  await expect(page.getByRole('button', { name: '传输并导入镜像' })).toHaveCount(0);
});

test('取消镜像传输时不导出、不上传、不导入', async ({ page }) => {
  await setupImageTransfer(page);
  await page.getByRole('button', { name: '本机拉取并传入' }).click();
  await page.getByRole('dialog').getByRole('button', { name: '取消', exact: true }).click();
  await expect(page.getByRole('button', { name: '本机拉取并传入' })).toBeEnabled();
  expect(await page.evaluate(() => window.__deployTestCalls.filter((item) =>
    ['prepare_local_deployment_images', 'upload_remote_image_bundle', 'import_remote_image_bundle'].includes(item.command)))).toEqual([]);
});

for (const [label, options, imports] of [
  ['导出失败', { exportFailed: true }, 0],
  ['上传失败', { failCommand: 'upload_remote_image_bundle' }, 0],
  ['导入失败', { failCommand: 'import_remote_image_bundle' }, 1],
  ['导入后仍缺失', { stillMissing: true }, 1]
]) {
  test(`镜像${label}时保留缺失状态并允许重试`, async ({ page }) => {
    await setupImageTransfer(page, options);
    await page.getByRole('button', { name: '本机拉取并传入' }).click();
    await page.getByRole('dialog').getByRole('button', { name: '拉取并传入', exact: true }).click();
    await expect(page.locator('.step-card .el-alert--error')).toBeVisible();
    await expect(page.getByRole('button', { name: '下一步：部署版本' })).toBeDisabled();
    await expect(page.getByRole('button', { name: '本机拉取并传入' })).toBeEnabled();
    await expect(
      page.getByText('本机镜像已按目标架构传入并导入，目标镜像核对通过。', { exact: true })
    ).toBeHidden();
    await expect(page.locator('.step-card .el-alert--error')).not.toContainText('Error:');
    expect(await page.evaluate(() => window.__deployTestCalls.filter((item) =>
      item.command === 'import_remote_image_bundle').length)).toBe(imports);
    await page.getByText('查看镜像处理详情', { exact: true }).click();
    await expect(page.locator('.image-diagnostics pre')).not.toBeEmpty();
  });
}

test('离线镜像包只传输缺失项对应文件，并在窄窗口中正常操作', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 1000 });
  await setupImageTransfer(page, { offline: true });
  await page.locator('.image-preparation').getByText('离线镜像包', { exact: true }).click();
  await page.getByRole('textbox', { name: '镜像介质目录' }).fill('/media/offline');
  await page.screenshot({ path: '/tmp/deploy-missing-images-narrow.png', fullPage: true, animations: 'disabled' });
  await page.getByRole('button', { name: '传输并导入镜像' }).click();
  await page.getByRole('dialog').getByRole('button', { name: '传输并导入', exact: true }).click();
  await expect(page.getByText('待处理镜像已导入，目标主机镜像核对通过。', { exact: true })).toBeVisible();
  const calls = await page.evaluate(() => window.__deployTestCalls);
  expect(calls.filter((item) => item.command === 'export_deployment_images')).toHaveLength(0);
  expect(calls.find((item) => item.command === 'upload_remote_image_bundle').args).toMatchObject({
    localDir: '/media/offline', files: ['api.tar.gz']
  });
});

test('导出期间展示阶段状态并阻止重复操作', async ({ page }) => {
  await setupImageTransfer(page, { pauseExport: true });
  await page.getByRole('button', { name: '本机拉取并传入' }).click();
  await page.getByRole('dialog').getByRole('button', { name: '拉取并传入', exact: true }).click();
  await expect(page.locator('.image-progress')).toContainText('正在本机按目标架构准备镜像');
  await expect(page.getByRole('button', { name: '本机拉取并传入' })).toBeDisabled();
  await expect(
    page.locator('.image-preparation').getByRole('button', { name: '重新检查', exact: true })
  ).toBeDisabled();
  await expect(page.getByRole('radio', { name: '离线镜像包', exact: true })).toBeDisabled();
  await page.evaluate(() => window.__finishImageExport());
  await expect(
    page.getByText('本机镜像已按目标架构传入并导入，目标镜像核对通过。', { exact: true })
  ).toBeVisible();
});

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
    .filter({ has: page.getByRole('heading', { name: '核对目标环境' }) });
  await expect(review).toHaveCount(1);
  await expect(review.locator('.status-row')).toContainText('检测到现有部署');
  await expect(review.locator('.status-row')).toContainText('制品版本 v1.4.0');
  await expect(review.locator('.status-row')).toContainText('2 个服务');
  await expect(review.locator('.compose-review-heading')).toContainText(
    '部署清单：deploy/docker-compose.yml'
  );
  await expect(review.getByText('配置有变化', { exact: false })).toBeVisible();

  // 只统计当前标签页的表格行，未激活的标签页仍留在 DOM 中。
  const activeRows = () => review.getByRole('tabpanel').locator('.el-table__row');
  await review.getByRole('tab', { name: '服务清单比对' }).click();
  await expect(activeRows()).toHaveCount(2);
  await expect(activeRows().nth(0)).toContainText('web');
  await expect(activeRows().nth(0)).toContainText('demo/web:1.4.0');
  await expect(activeRows().nth(0)).toContainText('demo/web:1.3.0');
  await expect(activeRows().nth(0)).toContainText('有变化');
  await expect(activeRows().nth(0)).toContainText('运行中');
  await expect(activeRows().nth(1)).toContainText('新增');
  await expect(activeRows().nth(1)).toContainText('未创建');

  await review.getByRole('tab', { name: '制品文件' }).click();
  await expect(activeRows()).toHaveCount(1);
  await expect(activeRows().nth(0)).toContainText('编排文件');
  await expect(activeRows().nth(0)).toContainText('已存在');

  await review.getByRole('tab', { name: '镜像与修复' }).click();
  await expect(activeRows().nth(0)).toContainText('demo/web:1.4.0');
  await expect(activeRows().nth(0)).toContainText('已就绪');
  await expect(activeRows().nth(1)).toContainText('demo/api:1.4.0');
  await expect(activeRows().nth(1)).toContainText('缺失');
  await expect(activeRows().nth(1)).toContainText('目标 Docker 中没有该镜像');

  await expect(review.locator('.image-preparation')).toContainText('准备待处理镜像');
  await expect(review.getByRole('radio', { name: '从镜像仓库拉取', exact: true })).toBeChecked();
  await expect(review.getByRole('button', { name: '拉取 1 个镜像' })).toBeVisible();
  await expect(review.getByRole('button', { name: '下一步：部署版本' })).toBeDisabled();
  await expect(review.locator('.next-blocked')).toContainText('仍有 1 个镜像未就绪');

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

test('清单比对步骤在镜像齐全时显示就绪状态并允许继续', async ({ page }) => {
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
      unreadyImages: [],
      ready: true,
      canIncrementalUpdate: true,
      images: demoDeploymentReview.images.map((item) => ({
        ...item,
        status: 'ready',
        platform: 'linux/amd64',
        detail: '目标 Docker 已具备该镜像',
        pullable: false
      })),
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
    .filter({ has: page.getByRole('heading', { name: '核对目标环境' }) });
  await expect(review.locator('.status-row')).toContainText('2 个服务');
  await expect(review.getByText('配置有变化', { exact: false })).toBeHidden();
  await expect(review.locator('.image-preparation')).toContainText('镜像已就绪');
  await expect(review.getByRole('button', { name: '下一步：部署版本' })).toBeEnabled();
  await review.getByRole('tab', { name: '服务清单比对' }).click();
  const serviceRows = review.getByRole('tabpanel').locator('.el-table__row');
  await expect(serviceRows).toHaveCount(2);
  await expect(serviceRows.nth(0)).toContainText('一致');
  await expect(serviceRows.nth(1)).toContainText('一致');

  expect(problems).toEqual([]);
});

test('制品文件缺失时提供写入入口，路径异常时阻止继续', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));
  page.on('console', (message) => {
    if (['error', 'warning'].includes(message.type())) problems.push(message.text());
  });

  const missingReview = {
    ...demoDeploymentReview,
    ready: true,
    canIncrementalUpdate: true,
    missingImages: [],
    unreadyImages: [],
    images: demoDeploymentReview.images.map((item) => ({
      ...item,
      status: 'ready',
      platform: 'linux/amd64',
      detail: '目标 Docker 已具备该镜像',
      pullable: false
    })),
    artifacts: [
      {
        ...demoDeploymentReview.artifacts[0],
        status: 'missing',
        actualBytes: 0,
        actualFiles: 0
      }
    ],
    artifactIssues: ['目标部署缺少 1 个制品文件，下一步完整版本部署会写入交付包声明的全部制品。']
  };
  const repairPlan = {
    ...demoBundlePlan,
    components: [{
      key: 'config', label: '部署配置', services: [], artifacts: [{ key: 'compose' }]
    }]
  };
  await useDesktopBackend(page, {
    hosts: [localHost],
    projects: [localProject],
    activeProjectId: localProject.id,
    localEnv,
    dialogPath: demoPreparedBundle.dir,
    preparedBundle: demoPreparedBundle,
    bundlePlan: repairPlan,
    deploymentReview: missingReview
  });
  await openReviewStep(page);
  await page.getByRole('tab', { name: '制品文件' }).click();
  await expect(page.getByRole('tabpanel').locator('.el-table__row').first()).toContainText('缺失');
  const writeButton = page.getByRole('button', { name: '下一步：部署完整版本' });
  await expect(writeButton).toBeEnabled();
  await writeButton.click();
  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('6部署版本');
  await expect(page.getByRole('button', { name: '开始完整版本部署' })).toBeEnabled();
  await expect(page.locator('.deployment-actions')).toHaveCount(0);
  await expect(page.getByRole('checkbox')).toHaveCount(0);
  await expect(page.getByText('部署失败时会停止新版本', { exact: false })).toBeVisible();

  const invalidReview = {
    ...missingReview,
    ready: false,
    canIncrementalUpdate: false,
    artifacts: [{ ...missingReview.artifacts[0], status: 'invalid' }],
    artifactIssues: ['有 1 个制品路径与交付包声明的类型不一致或包含符号链接，请在目标机处理该路径后重新检查。'],
    issues: ['有 1 个制品文件无法核对或路径异常，请处理目标目录后重新检查。']
  };
  await page.evaluate((review) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      if (command === 'inspect_deployment_review') {
        window.__deployTestCalls.push({ command, args });
        return structuredClone(review);
      }
      return invoke(command, args);
    };
  }, invalidReview);
  await page.getByRole('button', { name: '返回清单比对' }).click();
  await page.getByRole('tab', { name: '制品文件' }).click();
  await expect(page.getByRole('tabpanel').locator('.el-table__row').first()).toContainText('路径异常');
  await expect(page.getByRole('button', { name: '下一步：部署版本' })).toBeDisabled();
  await expect(page.locator('.next-blocked')).toContainText('制品文件无法核对或路径异常');
  await expect(
    page.getByRole('tabpanel', { name: '制品文件' }).getByRole('button', { name: '重新检查', exact: true })
  ).toBeVisible();
  expect(problems).toEqual([]);
});

test('部署版本步骤展示完整部署范围与恢复说明', async ({ page }) => {
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
      ready: true,
      canIncrementalUpdate: true,
      missingImages: [],
      unreadyImages: [],
      images: demoDeploymentReview.images.map((item) => ({
        ...item,
        status: 'ready',
        platform: 'linux/amd64',
        detail: '目标 Docker 已具备该镜像',
        pullable: false
      }))
    }
  });

  await openReviewStep(page);

  const review = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '核对目标环境' }) });
  await review.getByRole('button', { name: '下一步：部署版本' }).click();

  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('6部署版本');
  const step = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '部署完整版本' }) });
  await expect(step.locator('.desc')).toContainText('部署目录 /srv/deploy-demo');
  await expect(step.locator('.desc')).toContainText('制品版本 v1.4.0');
  await expect(step.locator('.desc')).toContainText('全部服务');

  await expect(step.locator('.summary-item').filter({ hasText: '部署方式' })).toContainText('完整版本部署');
  await expect(step.locator('.summary-item').filter({ hasText: '失败处理' })).toContainText('自动恢复上一版本');
  await expect(step.getByText('Web 控制台', { exact: true })).toBeVisible();
  await expect(step.getByText('接口服务', { exact: true })).toBeVisible();
  await expect(step.getByText('demo-data', { exact: true })).toBeVisible();
  await expect(step.getByText('deploy/backups', { exact: true })).toBeVisible();
  await expect(step.getByRole('checkbox')).toHaveCount(0);
  await expect(step.getByText('更新前备份')).toHaveCount(0);

  await expect(step.getByRole('button', { name: '执行预检' })).toBeEnabled();
  await expect(step.getByRole('button', { name: '开始完整版本部署' })).toBeEnabled();
  await expect(step.getByRole('button', { name: '查看集群监控' })).toHaveCount(0);

  await step.getByRole('button', { name: '返回清单比对' }).click();
  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('5清单比对');

  expect(problems).toEqual([]);
});

test('未完成事务阻止新部署，恢复失败时要求人工处理', async ({ page }, testInfo) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));
  const failedTransaction = {
    id: 'txn-1',
    state: 'failed',
    phase: 'starting',
    error: '新版本服务未就绪',
    snapshotDir: '/srv/deploy-demo/.deploy-desktop/transactions/txn-1',
    failureStage: 'starting',
    previousVersion: 'v1.3.0',
    version: 'v1.4.0'
  };
  await useDesktopBackend(page, {
    hosts: [localHost],
    projects: [localProject],
    activeProjectId: localProject.id,
    localEnv,
    dialogPath: demoPreparedBundle.dir,
    preparedBundle: demoPreparedBundle,
    bundlePlan: demoBundlePlan,
    deploymentTransaction: failedTransaction,
    deploymentReview: {
      ...demoDeploymentReview,
      ready: true,
      canIncrementalUpdate: true,
      missingImages: [],
      unreadyImages: [],
      images: demoDeploymentReview.images.map((item) => ({
        ...item,
        status: 'ready',
        platform: 'linux/amd64',
        detail: '目标 Docker 已具备该镜像',
        pullable: false
      }))
    }
  });

  await openReviewStep(page);
  const review = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '核对目标环境' }) });
  await review.getByRole('button', { name: '下一步：部署版本' }).click();
  const step = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '部署完整版本' }) });
  await expect(step.locator('.el-alert--error')).toContainText('未完成的部署事务');
  await expect(step.getByRole('button', { name: '开始完整版本部署' })).toHaveCount(0);
  await expect(step.getByRole('button', { name: '恢复上一版本' }).first()).toBeEnabled();

  await page.evaluate((transaction) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      if (command === 'inspect_deployment_transaction') {
        return { transaction: { ...transaction, state: 'recovery_failed', phase: 'recovery_failed', recoveryError: '数据卷恢复失败' } };
      }
      return invoke(command, args);
    };
  }, failedTransaction);
  await step.getByRole('button', { name: '返回清单比对' }).click();
  await review.getByRole('button', { name: '下一步：部署版本' }).click();
  const blocked = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '部署完整版本' }) });
  await expect(blocked.locator('.el-alert--error').first()).toContainText('自动恢复未完成');
  await expect(blocked.getByRole('button', { name: '恢复上一版本' })).toHaveCount(0);
  await expect(blocked).toContainText('请联系工程师完成人工恢复');
  await expect(blocked.getByText('demo-data', { exact: true })).toBeVisible();
  await page.locator('main.content-panel').evaluate((element) => element.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('manual-recovery.png'), fullPage: true, animations: 'disabled' });
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      if (command === 'inspect_deployment_transaction') throw new Error('部署事务记录格式无效');
      return invoke(command, args);
    };
  });
  await blocked.getByRole('button', { name: '返回清单比对' }).click();
  await review.getByRole('button', { name: '下一步：部署版本' }).click();
  await expect(step.getByRole('button', { name: '开始完整版本部署' })).toBeDisabled();
  await expect(step.locator('.el-alert--error')).toContainText('读取部署事务失败');
  expect(problems).toEqual([]);
});

test('新版本失败并恢复上一版本时显示恢复结果与事务快照', async ({ page }, testInfo) => {
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
      ready: true,
      canIncrementalUpdate: true,
      missingImages: [],
      unreadyImages: [],
      images: demoDeploymentReview.images.map((item) => ({
        ...item,
        status: 'ready',
        platform: 'linux/amd64',
        detail: '目标 Docker 已具备该镜像',
        pullable: false
      }))
    }
  });
  await openReviewStep(page);
  const review = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '核对目标环境' }) });
  await review.getByRole('button', { name: '下一步：部署版本' }).click();
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      if (command === 'deploy_local_version' && !args.dryRun) {
        return {
          success: false,
          recovered: true,
          recoveryFailed: false,
          stage: 'starting',
          version: 'v1.4.0',
          previousVersion: 'v1.3.0',
          transactionId: 'txn-recovered',
          snapshotDir: '/srv/deploy-demo/.deploy-desktop/transactions/txn-recovered',
          error: '新版本服务未就绪',
          output: '新版本部署失败，已恢复上一版本 v1.3.0',
          durationSecs: 42,
          status: null
        };
      }
      return invoke(command, args);
    };
  });
  const step = page
    .locator('.step-card')
    .filter({ has: page.getByRole('heading', { name: '部署完整版本' }) });
  await step.getByRole('button', { name: '开始完整版本部署' }).click();
  await expect(page.getByRole('dialog', { name: '确认完整版本部署' })).toContainText('demo-data');
  await page.getByRole('dialog', { name: '确认完整版本部署' })
    .getByRole('button', { name: '开始部署' }).click();
  await expect(step.locator('.execution-result')).toContainText('新版本部署失败，已恢复上一版本');
  await expect(step.locator('.execution-result')).toContainText('v1.3.0');
  await expect(step.getByText('事务快照：', { exact: false })).toContainText('txn-recovered');
  await expect(page.getByRole('dialog', { name: '确认完整版本部署' })).toBeHidden();
  await page.locator('main.content-panel').evaluate((element) => element.scrollTo(0, 0));
  await page.screenshot({ path: testInfo.outputPath('recovered-version.png'), fullPage: true, animations: 'disabled' });
  expect(problems).toEqual([]);
});

for (const scenario of [
  { name: '成功', success: true, recoveryFailed: false, state: 'committed', started: true, title: '完整版本部署完成' },
  { name: '自动恢复失败', success: false, recoveryFailed: true, state: 'recovery_failed', started: true, title: '新版本失败，自动恢复未完成' },
  { name: '停服前停止', success: false, recoveryFailed: false, state: 'failed', started: false, title: '部署未完成' }
]) {
  test(`完整版本部署结果：${scenario.name}`, async ({ page }) => {
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
        ready: true,
        missingImages: [],
        unreadyImages: [],
        images: demoDeploymentReview.images.map((image) => ({ ...image, status: 'ready', platform: 'linux/amd64' }))
      }
    });
    await openReviewStep(page);
    await page.getByRole('button', { name: '下一步：部署版本' }).click();
    await page.evaluate((item) => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
        if (command === 'deploy_local_version' && !args.dryRun) {
          return {
            success: item.success,
            recovered: false,
            recoveryFailed: item.recoveryFailed,
            previousVersion: 'v1.3.0',
            transactionId: 'txn-result',
            snapshotDir: '/srv/deploy-demo/.deploy-desktop/transactions/txn-result/snapshot',
            error: item.success ? null : '部署阶段失败，详情见日志',
            output: item.success ? '部署完成' : '诊断详情',
            status: { id: 'txn-result', state: item.state, hadDeployment: true, deploymentStarted: item.started }
          };
        }
        if (command === 'inspect_deployment_review') {
          return {
            ready: true,
            targetChecked: true,
            artifacts: [],
            services: item.success ? [{ name: 'web', containerState: 'running', containerHealth: 'healthy' }] : [],
            images: [],
            checkedAt: new Date().toISOString()
          };
        }
        return invoke(command, args);
      };
    }, scenario);
    const step = page.locator('.step-card').filter({ has: page.getByRole('heading', { name: '部署完整版本' }) });
    await step.getByRole('button', { name: '开始完整版本部署' }).click();
    await page.getByRole('dialog', { name: '确认完整版本部署' }).getByRole('button', { name: '开始部署' }).click();
    await expect(step.locator('.execution-result')).toContainText(scenario.title);
    if (scenario.success) {
      await expect(step.getByRole('button', { name: '查看集群监控' })).toBeVisible();
      await expect(step.getByRole('button', { name: '开始完整版本部署' })).toBeDisabled();
    } else if (scenario.recoveryFailed) {
      await expect(step).toContainText('请联系工程师完成人工恢复');
      await expect(step.getByRole('button', { name: '恢复上一版本' })).toHaveCount(0);
    } else {
      await expect(step.locator('.execution-result')).toContainText('修改目标前停止');
      await expect(step.getByRole('button', { name: '恢复上一版本' }).first()).toBeVisible();
    }
  });
}
