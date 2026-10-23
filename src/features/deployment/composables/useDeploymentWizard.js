import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { ElMessageBox } from 'element-plus';
import { selectLocalPath as open } from '../../../api/tauri.js';
import { connectionInventory } from '../../connections/state.js';
import { useDesktopProgress } from '../../../shared/composables/useDesktopProgress.js';
import {
  dockerComposeAction,
  remoteDockerComposeAction,
  installBundle,
  inspectBundlePlan,
  inspectDeploymentReview,
  isDesktopApp,
  onImageLibraryProgress,
  onImageManagerProgress
} from '../../../api/tauri.js';
import { validInstallDir, createDeploymentContext, contextTarget } from '../domain/context.js';
import {
  componentLabels,
  componentOptions as planComponentOptions,
  deploymentComponents,
  servicesAreReady
} from '../domain/components.js';
import { safeText, failureText } from '../domain/errors.js';
import { invokeIncremental } from '../services/operations.js';
import { prepareImagePackages, importImagePackages } from '../services/imagePreparation.js';
import { useEnvironmentCheck } from './useEnvironmentCheck.js';
import { useExecutionFeedback } from './useExecutionFeedback.js';
import { useHostAccess } from './useHostAccess.js';

export function useDeploymentWizard(config, emit) {
  const steps = ['选择项目', '检测环境', '选择制品', '解压制品', '清单比对', '选择更新'];

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
  const imageSource = ref(isRemote.value ? 'docker' : 'offline');
  const imagePhase = ref(''),
    imageProgress = ref(null),
    imageDiagnostics = ref(''),
    imageResult = ref('');

  watch(isRemote, (remote) => { imageSource.value = remote ? 'docker' : 'offline'; });
  const receiveImageProgress = (event) => {
    if (importingImages.value && event.phase !== 'done') {
      imageProgress.value = { percent: Math.max(0, Math.min(100, Number(event.percent) || 0)),
        message: safeText(event.message) };
    }
  };
  useDesktopProgress(onImageLibraryProgress, receiveImageProgress);
  useDesktopProgress(onImageManagerProgress, receiveImageProgress);

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

  const hostAccess = useHostAccess({ config, targetLabel, desktopGuard });
  const { testingSsh, settingKey, sshMessage, sshOk, testConnection, setupAccess } = hostAccess;

  const executionFeedback = useExecutionFeedback();

  const environment = useEnvironmentCheck({
    config,
    isRemote,
    targetLabel,
    desktopGuard,
    onInvalidateReview: () => {
      review.value = null;
      clearExecution();
    }
  });
  const {
    envData,
    envError,
    checkingEnv,
    installingDocker,
    environmentReady,
    checkEnvironment,
    installDocker
  } = environment;

  const busy = computed(
    () =>
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

  const componentOptions = computed(() => planComponentOptions(bundlePlan.value));
  const componentLabelMap = computed(() => componentLabels(bundlePlan.value));

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
    disposed = false;

  // Read-only SSH checks may continue after leaving; destructive work keeps its guard.
  const navigationBlocked = computed(() => settingKey.value ||
    installingDocker.value || artifactBusy.value || importingImages.value ||
    executing.value || preflighting.value);
  watch(navigationBlocked, (value) => emit('busy-change', value), { flush: 'sync' });

  onBeforeUnmount(() => {
    executionFeedback.stop();
    disposed = true;
    revision += 1;
    environment.dispose();
    emit('busy-change', false);
  });

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
    imageResult.value = '';
    imageDiagnostics.value = '';
    clearExecution();
  }

  // 只有部署目标真的变了才需要重来：绑定交付包会刷新项目上下文并回写连接信息，
  // 逐项比较可避免这种同目标刷新把已经走过的步骤清空。
  const connectionKeys = [
    'projectId',
    'activeHostId',
    'mode',
    'remoteHost',
    'remotePort',
    'remoteUser',
    'remoteKey',
    'installDir'
  ];
  const connectionSnapshot = () => connectionKeys.map((key) => String(config[key] ?? ''));
  let lastConnection = connectionSnapshot();

  watch(connectionSnapshot, (next) => {
    const changed = next.some((value, index) => value !== lastConnection[index]);
    lastConnection = next;
    if (!changed || executing.value) return;
    environment.invalidate();
    candidate.value = null;
    preparedBundle.value = null;
    clearReview();
    envData.value = null;
    envError.value = '';
    hostAccess.reset();
    if (activeStep.value > 0) activeStep.value = 0;
  });

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
    imageResult.value = '';
    imageDiagnostics.value = '';
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
      if (current === revision && !disposed) reviewError.value = failureText(error, '清单比对');
    } finally {
      if (current === revision && !disposed) checkingReview.value = false;
    }
  }

  async function pickImageDirectory() {
    try {
      desktopGuard();
      const directory = await open({
        directory: true,
        multiple: false,
        title: isRemote.value ? '选择离线镜像包目录' : '选择含 images/ 的离线介质目录'
      });
      if (typeof directory === 'string') imageMediaDir.value = directory;
    } catch (error) {
      reviewError.value = failureText(error, '选择镜像目录');
    }
  }

  async function prepareImages() {
    if (busy.value || !preparedBundle.value || !review.value?.missingImages?.length) return;
    const current = revision;
    const context = Object.freeze({
      source: imageSource.value,
      bundleDir: preparedBundle.value.dir,
      imageDir: imageMediaDir.value.trim() || preparedBundle.value.dir,
      installDir: config.installDir,
      label: targetLabel.value,
      target: target() ? Object.freeze({ ...target() }) : null,
      references: Object.freeze([...new Set(review.value.missingImages)]),
      remoteDir: `${config.installDir.replace(/\/+$/, '')}/bundle-images-${crypto.randomUUID()}`
    });
    importingImages.value = true;
    reviewError.value = '';
    imageDiagnostics.value = '';
    imageResult.value = '';
    imageProgress.value = null;
    clearExecution();
    const setImagePhase = (message) => {
      if (disposed || current !== revision) throw new Error('部署目标已变化，请重新选择项目并比对');
      imagePhase.value = message;
      imageProgress.value = null;
    };
    try {
      desktopGuard();
      try {
        await ElMessageBox.confirm(
          `目标：${context.label}。将${context.source === 'docker' ? '从本机 Docker 导出' : '从离线目录读取'} ${context.references.length} 个缺失镜像${
            context.target ? `，传输到 ${context.remoteDir}/images 后解压并导入 Docker` : '并导入本机 Docker'
          }。镜像包会保留并占用磁盘空间，运行中的服务不会重启。是否继续？`,
          '确认导入镜像',
          { type: 'warning', confirmButtonText: context.target ? '传输并导入' : '导入', cancelButtonText: '取消', closeOnClickModal: false }
        );
      } catch {
        return;
      }
      if (disposed || current !== revision) return;
      const packages = await prepareImagePackages(context, setImagePhase);
      await importImagePackages(context, packages, setImagePhase);
      setImagePhase('正在重新核对目标主机镜像');
      await loadReview();
      if (!review.value) throw new Error('镜像导入后未能确认目标状态，请重新比对');
      if (review.value.images?.some((image) => image.status !== 'ready') ||
          context.references.some((reference) => !review.value.images?.some((image) =>
            image.imageRef === reference && image.status === 'ready'))) {
        throw new Error('导入后仍有镜像缺失或未确认，请确认镜像包包含清单要求的完整镜像地址与摘要后重新比对');
      }
      imageResult.value = '缺失镜像已导入，目标主机镜像核对通过。';
    } catch (error) {
      reviewError.value = failureText(safeText(error?.message || error), '准备镜像');
      imageDiagnostics.value = safeText(error?.message || error);
    } finally {
      importingImages.value = false;
      imagePhase.value = '';
      imageProgress.value = null;
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
        : `替换${context.components.map((key) => componentLabelMap.value[key] || key).join('、')}并重启受影响的服务${
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
      executionFeedback.start((message) => {
        phase.value = message;
      });
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
      executionFeedback.stop();
    }
  }

  function executionContext() {
    return createDeploymentContext({ connection: config, remote: isRemote.value, bundle: preparedBundle.value,
      components: selectedComponents.value, backup: backup.value, action: deploymentAction.value,
      label: targetLabel.value, imageDir: imageMediaDir.value,
      allowedComponents: deploymentComponents(bundlePlan.value) });
  }

  const { progressStep, progressPercent, progressMessage, elapsedText } = executionFeedback;

  return { steps, activeStep, selectedHost, isRemote, targetLabel, hostReady, testingSsh, settingKey, sshMessage, sshOk, envData, envError, checkingEnv, installingDocker, artifactPicker, candidate, preparedBundle, artifactBusy, review, reviewError, checkingReview, reviewTab, imageMediaDir, imageSource, imagePhase, imageProgress, imageDiagnostics, imageResult, importingImages, deploymentAction, selectedComponents, backup, executing, preflighting, preflightPassed, phase, operationError, diagnostics, result, busy, environmentReady, componentOptions, canExecute, artifactChosen, artifactPrepared, testConnection, setupAccess, checkEnvironment, installDocker, nextStep, previousStep, loadReview, pickImageDirectory, prepareImages, runPreflight, executeDeployment, progressStep, progressPercent, progressMessage, elapsedText };
}
