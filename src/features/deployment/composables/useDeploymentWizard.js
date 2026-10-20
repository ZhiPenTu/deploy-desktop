import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { ElMessageBox } from 'element-plus';
import { selectLocalPath as open } from '../../../api/tauri.js';
import { connectionInventory, useManagedHost } from '../../connections/state.js';
import {
  checkLocalEnv,
  checkRemoteEnv,
  installOfflineDockerEngine,
  installRemoteDockerEngine,
  dockerComposeAction,
  remoteDockerComposeAction,
  installBundle,
  inspectBundlePlan,
  inspectDeploymentReview,
  isDesktopApp,
  testManagedHostConnection,
  setupManagedHostAccess,
  scanLocalImageBundle,
  loadOfflineImages,
  uploadRemoteImageBundle,
  importRemoteImageBundle,
} from '../../../api/tauri.js';
import { validInstallDir, createDeploymentContext, contextTarget } from '../domain/context.js';
import { componentLabels, deploymentComponents, servicesAreReady } from '../domain/components.js';
import { safeText, failureText } from '../domain/errors.js';
import { invokeIncremental } from '../services/operations.js';

export function useDeploymentWizard(config, emit) {
  const steps = ['选择主机', '检测环境', '选择制品', '解压制品', '清单比对', '选择更新'];

  const activeStep = ref(0);

  const selectedHost = computed(() =>
    connectionInventory.hosts.find((host) => host.id === config.activeHostId)
  );

  const isRemote = computed(() => selectedHost.value?.kind !== 'local');

  const targetLabel = computed(() =>
    selectedHost.value
      ? isRemote.value
        ? `${selectedHost.value.name}（${config.remoteHost}）`
        : selectedHost.value.name === '本机'
        ? '本机'
        : `${selectedHost.value.name}（本机）`
      : '未选择主机'
  );

  const hostReady = computed(() =>
    Boolean(
      selectedHost.value &&
        validInstallDir(config.installDir, isRemote.value) &&
        (!isRemote.value || (config.remoteHost && config.remoteUser))
    )
  );

  const selectingHost = ref(false),
    testingSsh = ref(false),
    settingKey = ref(false),
    sshMessage = ref(''),
    sshOk = ref(false);

  const envData = ref(null),
    envError = ref(''),
    checkingEnv = ref(false),
    installingDocker = ref(false);

  const artifactPicker = ref(null),
    candidate = ref(null),
    preparedBundle = ref(null),
    artifactBusy = ref(false);

  const review = ref(null),
    reviewError = ref(''),
    checkingReview = ref(false),
    reviewTab = ref('services');

  const bundlePlan = ref(null),
    imageMediaDir = ref(''),
    importingImages = ref(false);

  const deploymentAction = ref('first'),
    selectedComponents = ref([]),
    backup = ref(true);

  const executing = ref(false),
    preflighting = ref(false),
    preflightPassed = ref(false),
    phase = ref(''),
    operationError = ref(''),
    diagnostics = ref(''),
    result = ref(null);

  const busy = computed(
    () =>
      selectingHost.value ||
      testingSsh.value ||
      settingKey.value ||
      checkingEnv.value ||
      installingDocker.value ||
      artifactBusy.value ||
      checkingReview.value ||
      importingImages.value ||
      executing.value ||
      preflighting.value
  );

  const environmentReady = computed(() =>
    Boolean(envData.value?.docker_running && envData.value?.compose_installed && !envError.value)
  );

  const componentOptions = computed(() => deploymentComponents(bundlePlan.value).map((key) => ({ key, label: componentLabels[key] })));

  const canExecute = computed(
    () =>
      hostReady.value &&
      environmentReady.value &&
      preparedBundle.value &&
      review.value?.ready &&
      (deploymentAction.value === 'first'
        ? review.value.canFirstDeploy
        : review.value.canIncrementalUpdate && selectedComponents.value.length > 0)
  );

  let revision = 0,
    envRevision = 0,
    disposed = false;

  watch(busy, (value) => emit('busy-change', value), { flush: 'sync' });

  onBeforeUnmount(() => {
    disposed = true;
    revision += 1;
    envRevision += 1;
    emit('busy-change', false);
  });

  const changeLabel = (value) =>
    ({ added: '新增', changed: '有变化', unchanged: '一致', removed: '制品中移除' }[value] || '未确认');

  const stateLabel = (value) =>
    ({
      running: '运行中',
      exited: '已停止',
      created: '已创建',
      restarting: '重启中',
      not_created: '未创建',
      missing: '未创建'
    }[value] || '未运行');

  const target = () =>
    isRemote.value
      ? {
          host: config.remoteHost,
          port: config.remotePort,
          user: config.remoteUser,
          keyPath: config.remoteKey || null
        }
      : null;

  function desktopGuard() {
    if (!isDesktopApp()) throw new Error('当前环境暂不可用，请在桌面应用中操作。');
  }

  function clearExecution() {
    preflightPassed.value = false;
    result.value = null;
    phase.value = '';
    operationError.value = '';
    diagnostics.value = '';
  }

  function clearReview() {
    revision += 1;
    review.value = null;
    reviewError.value = '';
    bundlePlan.value = null;
    clearExecution();
  }

  watch(
    () => [
      config.activeHostId,
      config.mode,
      config.remoteHost,
      config.remotePort,
      config.remoteUser,
      config.remoteKey,
      config.installDir
    ],
    () => {
      if (executing.value) return;
      envRevision += 1;
      clearReview();
      envData.value = null;
      envError.value = '';
      sshMessage.value = '';
      if (activeStep.value > 0) activeStep.value = 0;
      config.deployDir = config.installDir;
    }
  );

  watch(() => [deploymentAction.value, selectedComponents.value.join(','), backup.value], clearExecution);

  function artifactChosen(value) {
    candidate.value = value;
    preparedBundle.value = null;
    clearReview();
  }

  function artifactPrepared(value) {
    preparedBundle.value = value;
    clearReview();
  }

  async function selectHost(id) {
    if (busy.value) return;
    selectingHost.value = true;
    try {
      await useManagedHost(id);
    } catch (error) {
      sshOk.value = false;
      sshMessage.value = failureText(error, '切换主机');
    } finally {
      selectingHost.value = false;
    }
  }

  async function testConnection() {
    testingSsh.value = true;
    sshMessage.value = '';
    try {
      desktopGuard();
      const response = await testManagedHostConnection(config.activeHostId);
      sshOk.value = Boolean(response.success);
      sshMessage.value = response.success ? '连接正常' : '连接未通过，请检查凭证与网络。';
    } catch (error) {
      sshOk.value = false;
      sshMessage.value = failureText(error, '连接检查');
    } finally {
      testingSsh.value = false;
    }
  }

  async function setupAccess() {
    try {
      await ElMessageBox.confirm(
        `将为 ${targetLabel.value} 配置免密登录，使用该主机保存的凭证。`,
        '确认配置免密登录',
        { type: 'warning', confirmButtonText: '开始配置', cancelButtonText: '取消' }
      );
    } catch {
      return;
    }
    settingKey.value = true;
    try {
      desktopGuard();
      const response = await setupManagedHostAccess(config.activeHostId);
      sshOk.value = Boolean(response.success);
      sshMessage.value = response.success ? '免密登录已配置' : '配置未完成，请检查凭证后重试。';
    } catch (error) {
      sshOk.value = false;
      sshMessage.value = failureText(error, '配置免密登录');
    } finally {
      settingKey.value = false;
    }
  }

  async function checkEnvironment() {
    const current = ++envRevision;
    checkingEnv.value = true;
    envError.value = '';
    envData.value = null;
    review.value = null;
    clearExecution();
    try {
      desktopGuard();
      const response = isRemote.value
        ? await checkRemoteEnv(
            config.remoteHost,
            config.remotePort,
            config.remoteUser,
            config.remoteKey || null
          )
        : await checkLocalEnv(config.installDir);
      if (current === envRevision && !disposed) envData.value = response;
    } catch (error) {
      if (current === envRevision) envError.value = failureText(error, '环境检测');
    } finally {
      checkingEnv.value = false;
    }
  }

  async function installDocker() {
    try {
      await ElMessageBox.confirm(
        `将在 ${targetLabel.value} 安装并启动 Docker 引擎，可能修改系统服务配置。`,
        '确认安装 Docker',
        { type: 'warning', confirmButtonText: '安装', cancelButtonText: '取消' }
      );
    } catch {
      return;
    }
    installingDocker.value = true;
    try {
      desktopGuard();
      const response = isRemote.value
        ? await installRemoteDockerEngine(
            config.remoteHost,
            config.remotePort,
            config.remoteUser,
            config.remoteKey || null,
            config.mediaDir
          )
        : await installOfflineDockerEngine(config.mediaDir);
      if (!response.success) throw new Error(response.error_message || 'Docker 安装失败');
      await checkEnvironment();
    } catch (error) {
      envError.value = failureText(error, '安装 Docker');
    } finally {
      installingDocker.value = false;
    }
  }

  async function nextStep() {
    if (busy.value) return;
    if (activeStep.value === 0 && hostReady.value) {
      activeStep.value = 1;
      await checkEnvironment();
    } else if (activeStep.value === 1 && environmentReady.value) activeStep.value = 2;
    else if (activeStep.value === 2 && candidate.value) activeStep.value = 3;
    else if (activeStep.value === 3 && preparedBundle.value) {
      activeStep.value = 4;
      await loadReview();
    } else if (activeStep.value === 4 && review.value?.ready) {
      deploymentAction.value = review.value.hasDeployment ? 'incremental' : 'first';
      selectedComponents.value = componentOptions.value.map((component) => component.key);
      activeStep.value = 5;
    }
  }

  async function previousStep() {
    if (busy.value || activeStep.value === 0) return;
    activeStep.value -= 1;
    if (activeStep.value === 4) await loadReview();
  }

  async function loadReview() {
    if (!preparedBundle.value) return;
    const current = ++revision;
    checkingReview.value = true;
    review.value = null;
    reviewError.value = '';
    clearExecution();
    try {
      desktopGuard();
      const [checked, plan] = await Promise.all([
        inspectDeploymentReview(preparedBundle.value.dir, config.installDir, target(), imageMediaDir.value),
        inspectBundlePlan(preparedBundle.value.dir, target())
      ]);
      if (current !== revision || disposed) return;
      review.value = checked;
      bundlePlan.value = plan;
    } catch (error) {
      if (current === revision) reviewError.value = failureText(error, '清单比对');
    } finally {
      checkingReview.value = false;
    }
  }

  async function pickImageDirectory() {
    try {
      desktopGuard();
      const directory = await open({
        directory: true,
        multiple: false,
        title: '选择含 images/ 的离线介质目录'
      });
      if (typeof directory === 'string') imageMediaDir.value = directory;
    } catch (error) {
      reviewError.value = failureText(error, '选择镜像目录');
    }
  }

  async function prepareImages() {
    const root = imageMediaDir.value.trim() || preparedBundle.value?.dir;
    importingImages.value = true;
    reviewError.value = '';
    try {
      desktopGuard();
      const scan = await scanLocalImageBundle(root);
      const files = (scan.files || []).map((file) => file.name);
      if (!files.length) throw new Error('镜像目录中没有离线包');
      try {
        await ElMessageBox.confirm(
          `将向 ${targetLabel.value} 导入 ${files.length} 个镜像包。${
            isRemote.value ? '镜像包会先上传到部署目录。' : ''
          }此操作会占用磁盘空间。`,
          '确认导入镜像',
          { type: 'warning', confirmButtonText: '导入', cancelButtonText: '取消' }
        );
      } catch {
        return;
      }
      if (isRemote.value) {
        const remoteDir = `${config.installDir.replace(/\/+$/, '')}/bundle-images-${Date.now()}`;
        await uploadRemoteImageBundle(
          config.remoteHost,
          config.remotePort,
          config.remoteUser,
          config.remoteKey || null,
          root,
          remoteDir,
          files,
          false
        );
        await importRemoteImageBundle(
          config.remoteHost,
          config.remotePort,
          config.remoteUser,
          config.remoteKey || null,
          remoteDir,
          files
        );
      } else {
        await loadOfflineImages(root);
      }
      await loadReview();
    } catch (error) {
      reviewError.value = failureText(error, '导入镜像');
    } finally {
      importingImages.value = false;
    }
  }

  async function preflight(context) {
    preflightPassed.value = false;
    desktopGuard();
    const checked = await inspectDeploymentReview(
      context.bundleDir,
      context.installDir,
      contextTarget(context),
      context.imageDir
    );
    review.value = checked;
    if (
      !checked.ready ||
      (context.action === 'first' ? !checked.canFirstDeploy : !checked.canIncrementalUpdate)
    )
      throw new Error('清单、镜像或目标部署状态已变化，请返回重新比对');
    if (context.action === 'incremental') {
      const response = await invokeIncremental(context, true);
      diagnostics.value = safeText(response.output);
    }
    preflightPassed.value = true;
  }

  async function runPreflight() {
    if (!canExecute.value || busy.value) return;
    preflighting.value = true;
    operationError.value = '';
    preflightPassed.value = false;
    try {
      await preflight(executionContext());
    } catch (error) {
      operationError.value = failureText(error, '预检');
      diagnostics.value = safeText(error);
    } finally {
      preflighting.value = false;
    }
  }

  async function executeDeployment() {
    if (!canExecute.value || busy.value) return;
    let context;
    preflighting.value = true;
    operationError.value = '';
    result.value = null;
    try {
      context = executionContext();
      await preflight(context);
    } catch (error) {
      operationError.value = failureText(error, '预检');
      diagnostics.value = safeText(error);
      return;
    } finally {
      preflighting.value = false;
    }
    const action = context.action === 'first' ? '首次部署' : '增量更新';
    const scope =
      context.action === 'first'
        ? '写入交付包并启动全部清单服务'
        : `替换${context.components.map((key) => componentLabels[key]).join('、')}并重启受影响的服务${
            context.backup ? '，更新前保留备份' : '，本次不备份'
          }`;
    executing.value = true;
    try {
      try {
        await ElMessageBox.confirm(
          `目标：${context.label}；目录：${context.installDir}；版本：${
            context.version || '未标记'
          }。将${scope}。是否继续？`,
          `确认${action}`,
          {
            type: 'warning',
            confirmButtonText: `开始${action}`,
            cancelButtonText: '取消',
            closeOnClickModal: false
          }
        );
      } catch {
        return;
      }
      phase.value = `正在${action}，请等待执行结果`;
      let response;
      if (context.action === 'incremental') {
        response = await invokeIncremental(context, false);
      } else {
        phase.value = '正在写入交付包';
        const installed = await installBundle(
          context.remote ? 'remote' : 'local',
          context.host,
          context.port,
          context.user,
          context.keyPath,
          context.bundleDir,
          context.installDir
        );
        if (!installed.success) throw new Error(installed.error || '交付包写入失败');
        diagnostics.value = safeText(installed.output);
        phase.value = '正在启动清单中的服务';
        response = context.remote
          ? await remoteDockerComposeAction(
              context.host,
              context.port,
              context.user,
              context.keyPath,
              context.installDir,
              'up'
            )
          : await dockerComposeAction(context.installDir, 'up');
        if (!response.success) throw new Error(response.error || '服务启动失败');
      }
      diagnostics.value += `\n${safeText(response.output)}`;
      phase.value = '正在检查服务运行状态';
      const verified = await inspectDeploymentReview(
        context.bundleDir,
        context.installDir,
        contextTarget(context),
        context.imageDir
      );
      const ready = servicesAreReady(verified);
      result.value = {
        success: ready,
        pending: !ready,
        backupDir: response.backup_dir,
        message: ready
          ? '服务已运行，可前往集群监控检查业务状态。'
          : '执行命令已完成，部分服务的就绪状态仍需确认，请前往集群监控检查。'
      };
      phase.value = '';
    } catch (error) {
      result.value = {
        ...result.value,
        backupDir: error.result?.backup_dir || result.value?.backupDir,
        success: false,
        message: '执行未完成，请查看执行详情，处理后重新比对。'
      };
      operationError.value = failureText(error, action);
      diagnostics.value += `\n${safeText(error.result?.output || '')}\n${safeText(error)}`;
      phase.value = '';
    } finally {
      executing.value = false;
    }
  }

  function executionContext() {
    return createDeploymentContext({ connection: config, remote: isRemote.value, bundle: preparedBundle.value,
      components: selectedComponents.value, backup: backup.value, action: deploymentAction.value,
      label: targetLabel.value, imageDir: imageMediaDir.value });
  }

  return { steps, activeStep, selectedHost, isRemote, targetLabel, hostReady, selectingHost, testingSsh, settingKey, sshMessage, sshOk, envData, envError, checkingEnv, installingDocker, artifactPicker, candidate, preparedBundle, artifactBusy, review, reviewError, checkingReview, reviewTab, imageMediaDir, importingImages, deploymentAction, selectedComponents, backup, executing, preflighting, preflightPassed, phase, operationError, diagnostics, result, busy, environmentReady, componentOptions, canExecute, changeLabel, stateLabel, artifactChosen, artifactPrepared, selectHost, testConnection, setupAccess, checkEnvironment, installDocker, nextStep, previousStep, loadReview, pickImageDirectory, prepareImages, runPreflight, executeDeployment };
}
