import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { ElMessageBox } from 'element-plus';
import { selectLocalPath as open } from '../../../api/tauri.js';
import { connectionInventory } from '../../connections/state.js';
import { useDesktopProgress } from '../../../shared/composables/useDesktopProgress.js';
import {
  inspectDeploymentTransaction,
  inspectBundlePlan,
  inspectDeploymentReview,
  isDesktopApp,
  listRegistrySources,
  onImageLibraryProgress,
  onImageManagerProgress,
  recoverDeploymentTransaction
} from '../../../api/tauri.js';
import { validInstallDir, createDeploymentContext, contextTarget } from '../domain/context.js';
import {
  servicesAreReady
} from '../domain/components.js';
import { safeText, failureText } from '../domain/errors.js';
import {
  groupPullReferences,
  imagePreparationCounts,
  imageStatusLabel,
  isDigestReference,
  pendingImages,
  pullableImages
} from '../domain/images.js';
import { invokeVersionDeployment } from '../services/operations.js';
import {
  prepareImagePackages,
  prepareRelayImagePackages,
  importImagePackages,
  pullMissingImages,
  registerTransferredImages
} from '../services/imagePreparation.js';
import { useEnvironmentCheck } from './useEnvironmentCheck.js';
import { useExecutionFeedback } from './useExecutionFeedback.js';
import { useHostAccess } from './useHostAccess.js';

const DIGEST_TRANSFER_HINT =
  '离线包中的镜像无法与仓库摘要核对，请从镜像仓库拉取；局域网目标可选择本机拉取后传入';

export function useDeploymentWizard(config, emit) {
  const steps = ['选择项目', '检测环境', '选择制品', '解压制品', '清单比对', '部署版本'];

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
    reviewTab = ref('images');

  const bundlePlan = ref(null),
    imageMediaDir = ref(''),
    importingImages = ref(false);
  const imageSource = ref('pull');
  const imageRegistrySources = ref([]),
    imagePullSelections = ref({}),
    imageRegistryLoadError = ref('');
  const imageOperationRevision = ref(0);
  const imagePhase = ref(''),
    imageProgress = ref(null),
    imageDiagnostics = ref(''),
    imageResult = ref('');

  const imagePullGroups = computed(() =>
    groupPullReferences(
      review.value?.images || [],
      imageRegistrySources.value,
      imagePullSelections.value
    )
  );
  const imagePending = computed(() => pendingImages(review.value?.images || []));
  const imagePullable = computed(() => pullableImages(review.value?.images || []));
  const pendingDigestImages = computed(() =>
    imagePending.value.filter((image) => isDigestReference(image.imageRef))
  );
  const imageCounts = computed(() => imagePreparationCounts(review.value?.images || []));
  const imageBlockedCount = computed(
    () => imagePending.value.length - imagePullable.value.length
  );
  const imageNextBlockedReason = computed(() => {
    if (checkingReview.value) return '正在核对目标环境，请稍候';
    if (importingImages.value) return '正在准备镜像，请稍候';
    if (!review.value) return '请先完成清单比对';
    if (review.value.ready) return '';
    if (imagePending.value.length) {
      return `仍有 ${imagePending.value.length} 个镜像未就绪，请先在上方处理`;
    }
    if (!review.value.dockerRunning) return '目标 Docker 未运行，请返回环境检测启动 Docker';
    if (!review.value.composeAvailable) return '目标缺少 Docker Compose，请先准备运行环境';
    return review.value.issues?.[0] || '目标环境尚未就绪';
  });

  function setImagePullSelection(registry, patch) {
    if (!registry) return;
    const previous = imagePullSelections.value[registry] || {};
    const next = { ...previous, ...patch };
    if (Object.prototype.hasOwnProperty.call(patch, 'sourceId')) {
      next.credential = '';
    }
    imagePullSelections.value = { ...imagePullSelections.value, [registry]: next };
  }

  function clearImageCredentials() {
    imagePullSelections.value = Object.fromEntries(
      Object.entries(imagePullSelections.value).map(([registry, value]) => [
        registry,
        { ...value, credential: '' }
      ])
    );
  }

  watch(isRemote, () => {
    imageSource.value = 'pull';
    imagePullSelections.value = {};
  });
  watch(activeStep, (step) => {
    if (step !== 4) clearImageCredentials();
  });
  watch(imageSource, (value) => {
    if (value === 'offline') clearImageCredentials();
  });
  watch(
    () => review.value?.images?.map((image) => image.imageRef).join('\n') || '',
    () => {
      if (!review.value) return;
      const registries = new Set(imagePullGroups.value.map((group) => group.registry));
      imagePullSelections.value = Object.fromEntries(
        Object.entries(imagePullSelections.value).filter(([registry]) =>
          registries.has(registry)
        )
      );
    }
  );
  const receiveImageProgress = (event) => {
    if (importingImages.value && event.phase !== 'done' &&
        imageOperationRevision.value !== 0) {
      imageProgress.value = { percent: Math.max(0, Math.min(100, Number(event.percent) || 0)),
        message: safeText(event.message) };
    }
  };
  useDesktopProgress(onImageLibraryProgress, receiveImageProgress);
  useDesktopProgress(onImageManagerProgress, receiveImageProgress);

  const transaction = ref(null),
    recoveringTransaction = ref(false);

  const executing = ref(false),
    preflighting = ref(false),
    preflightPassed = ref(false),
    preflightMessage = ref(''),
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

  const artifactSelectionIssue = computed(() => '');
  const transactionReadError = ref('');
  const checkingTransaction = ref(false);
  let transactionReadSequence = 0;

  const canExecute = computed(
    () =>
      hostReady.value &&
      environmentReady.value &&
      preparedBundle.value &&
      !checkingTransaction.value &&
      !transactionReadError.value &&
      !(
        transaction.value &&
        !['committed', 'recovered'].includes(transaction.value.state)
      ) &&
      review.value?.ready &&
      !artifactSelectionIssue.value
  );

  let revision = 0,
    imageOperationSequence = 0,
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
    clearImageCredentials();
    imageOperationRevision.value = 0;
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
    preflightMessage.value = '';
    result.value = null;
    phase.value = '';
    operationError.value = '';
    diagnostics.value = '';
  }

  async function inspectTransaction() {
    const sequence = ++transactionReadSequence;
    if (!config.installDir?.trim()) {
      transaction.value = null;
      transactionReadError.value = '';
      checkingTransaction.value = false;
      return;
    }
    checkingTransaction.value = true;
    try {
      const response = await inspectDeploymentTransaction(config.installDir.trim(), target());
      if (sequence !== transactionReadSequence || disposed) return;
      if (operationError.value === transactionReadError.value) operationError.value = '';
      transactionReadError.value = '';
      transaction.value = response?.transaction
        ? {
            ...response.transaction,
            error: safeText(response.transaction.error),
            recoveryError: safeText(response.transaction.recoveryError)
          }
        : null;
    } catch (error) {
      if (sequence !== transactionReadSequence || disposed) return;
      transaction.value = null;
      transactionReadError.value = failureText(error, '读取部署事务', { remote: isRemote.value });
      operationError.value = transactionReadError.value;
    } finally {
      if (sequence === transactionReadSequence) checkingTransaction.value = false;
    }
  }

  function clearReview() {
    revision += 1;
    review.value = null;
    reviewError.value = '';
    bundlePlan.value = null;
    imageResult.value = '';
    imageDiagnostics.value = '';
    imagePullSelections.value = {};
    imageRegistryLoadError.value = '';
    imagePhase.value = '';
    imageProgress.value = null;
    imageOperationRevision.value = 0;
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
      activeStep.value = 5;
      await inspectTransaction();
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
    imageRegistryLoadError.value = '';
    try {
      desktopGuard();
      const [checked, plan, registry] = await Promise.all([
        inspectDeploymentReview(preparedBundle.value.dir, config.installDir, target(), imageMediaDir.value),
        inspectBundlePlan(preparedBundle.value.dir, target()),
        listRegistrySources()
          .then((sources) => ({ sources: Array.isArray(sources) ? sources : [], error: '' }))
          .catch((error) => ({
            sources: [],
            error: `读取镜像源失败：${safeText(error?.message || error) || '请稍后重试'}`
          }))
      ]);
      if (current !== revision || disposed) return;
      review.value = checked;
      bundlePlan.value = plan;
      imageRegistrySources.value = registry.sources;
      imageRegistryLoadError.value = registry.error;
      if (pendingImages(checked.images || []).length) reviewTab.value = 'images';
      await inspectTransaction();
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
    if (busy.value || !preparedBundle.value || !review.value) return;
    const pending = pendingImages(review.value.images || []);
    const pullable = pullableImages(review.value.images || []);
    const source = imageSource.value;
    const transferable =
      source === 'offline'
        ? pending.filter((image) => !isDigestReference(image.imageRef))
        : pullable;
    const references = transferable.map((image) => image.imageRef);
    if (!references.length) {
      reviewError.value = pendingDigestImages.value.length
        ? `${DIGEST_TRANSFER_HINT}：${pendingDigestImages.value
            .map((image) => image.imageRef)
            .join('、')}`
        : source === 'offline'
          ? '没有需要导入的镜像，请先重新检查目标环境'
          : '当前没有可通过拉取修复的镜像；请查看镜像原因，或改用离线镜像包';
      imageDiagnostics.value = '';
      imageResult.value = '';
      return;
    }
    if (source === 'relay' && !isRemote.value) {
      reviewError.value = '本机拉取后传入仅适用于远程目标主机';
      imageDiagnostics.value = '';
      imageResult.value = '';
      return;
    }
    const missingCredentials = imagePullGroups.value.filter(
      (group) => group.needsPassword && !group.credential
    );
    if (source !== 'offline' && missingCredentials.length) {
      reviewError.value = `请填写 ${missingCredentials
        .map((group) => group.label)
        .join('、')} 的镜像仓库密码`;
      imageDiagnostics.value = '';
      imageResult.value = '';
      return;
    }
    const credentials =
      source === 'offline'
        ? []
        : imagePullGroups.value
            .filter((group) => group.sourceId)
            .map((group) => ({
              registrySourceId: group.sourceId,
              credential: group.credential
            }));
    const operation = ++imageOperationSequence;
    imageOperationRevision.value = operation;
    const context = Object.freeze({
      source,
      bundleDir: preparedBundle.value.dir,
      imageDir: imageMediaDir.value.trim() || preparedBundle.value.dir,
      installDir: config.installDir,
      label: targetLabel.value,
      target: target() ? Object.freeze({ ...target() }) : null,
      references: Object.freeze([...new Set(references)]),
      credentials: Object.freeze(credentials.map((item) => Object.freeze({ ...item }))),
      remoteDir: `${config.installDir.replace(/\/+$/, '')}/bundle-images-${crypto.randomUUID()}`
    });
    importingImages.value = true;
    reviewError.value = '';
    imageDiagnostics.value = '';
    imageResult.value = '';
    imageProgress.value = null;
    clearExecution();
    const setImagePhase = (message) => {
      if (disposed || imageOperationRevision.value !== operation) {
        throw new Error('部署目标已变化，请重新选择项目并比对');
      }
      imagePhase.value = message;
      imageProgress.value = null;
    };
    try {
      desktopGuard();
      try {
        const sourceText =
          context.source === 'relay'
            ? '先在本机按目标架构拉取并导出'
            : context.source === 'offline'
              ? '从离线目录读取'
              : context.target
                ? '在目标主机按清单地址拉取'
                : '在本机按清单地址拉取';
        const destinationText =
          context.source === 'pull'
            ? context.target
              ? `到${context.label}的 Docker`
              : '到本机 Docker'
            : context.source === 'relay'
              ? `，再通过 SSH 传输到 ${context.remoteDir}/images 后导入`
            : context.target
              ? `，传输到 ${context.remoteDir}/images 后解压并导入 Docker`
              : '并导入本机 Docker';
        const retentionText =
          context.source === 'pull'
            ? '拉取完成后会重新核对，运行中的服务不会重启。'
            : context.source === 'relay'
              ? '目标机无需访问镜像仓库；镜像包会保留并占用本机磁盘空间，运行中的服务不会重启。'
              : '镜像包会保留并占用磁盘空间，运行中的服务不会重启。';
        await ElMessageBox.confirm(
          `目标：${context.label}。将${sourceText} ${context.references.length} 个可处理镜像${destinationText}。${retentionText}是否继续？`,
          context.source === 'pull'
            ? '确认拉取镜像'
            : context.source === 'relay'
              ? '确认本机拉取并传入'
              : '确认导入镜像',
          {
            type: 'warning',
            confirmButtonText:
              context.source === 'pull'
                ? '开始拉取'
                : context.source === 'relay'
                  ? '拉取并传入'
                  : context.target
                    ? '传输并导入'
                    : '导入',
            cancelButtonText: '取消',
            closeOnClickModal: false
          }
        );
      } catch {
        return;
      }
      if (disposed || imageOperationRevision.value !== operation) return;
      if (context.source === 'pull') {
        await pullMissingImages(context, setImagePhase);
      } else if (context.source === 'relay') {
        const packages = await prepareRelayImagePackages(context, setImagePhase);
        await importImagePackages(context, packages, setImagePhase);
        await registerTransferredImages(context, packages, setImagePhase);
      } else {
        const packages = await prepareImagePackages(context, setImagePhase);
        await importImagePackages(context, packages, setImagePhase);
        await registerTransferredImages(context, packages, setImagePhase);
      }
      setImagePhase('正在重新核对目标镜像');
      await loadReview();
      if (!review.value) throw new Error('镜像准备后未能确认目标状态，请重新比对');
      const remaining = pendingImages(review.value.images || []);
      if (remaining.length) {
        const details = remaining
          .slice(0, 3)
          .map((image) => `${image.imageRef}（${imageStatusLabel(image.status)}）`)
          .join('、');
        const remainingDigest = remaining.filter((image) =>
          isDigestReference(image.imageRef)
        );
        throw new Error(
          `仍有 ${remaining.length} 个镜像未就绪：${details}。${
            context.source === 'offline' && remainingDigest.length
              ? `${DIGEST_TRANSFER_HINT}。`
              : '请查看镜像原因后重试。'
          }`
        );
      }
      imageResult.value =
        context.source === 'pull'
          ? '待处理镜像已拉取，目标镜像核对通过。'
          : context.source === 'relay'
            ? '本机镜像已按目标架构传入并导入，目标镜像核对通过。'
            : `待处理镜像已导入，${context.target ? '目标主机' : '目标'}镜像核对通过。`;
    } catch (error) {
      if (imageOperationRevision.value === operation && !disposed) {
        const message = safeText(error?.message || error);
        reviewError.value = message || failureText(message, '准备镜像');
        imageDiagnostics.value = message;
      }
    } finally {
      importingImages.value = false;
      imagePhase.value = '';
      imageProgress.value = null;
      clearImageCredentials();
      imageOperationRevision.value = 0;
    }
  }

  async function preflight(context) {
    preflightPassed.value = false;
    preflightMessage.value = '';
    desktopGuard();
    const checked = await inspectDeploymentReview(
      context.bundleDir,
      context.installDir,
      contextTarget(context),
      context.imageDir
    );
    review.value = checked;
    if (!checked.ready)
      throw new Error('清单、镜像或目标部署状态已变化，请返回重新比对');
    const response = await invokeVersionDeployment(context, true);
    diagnostics.value = safeText(response.output);
    preflightMessage.value = safeText(response.output);
    preflightPassed.value = true;
  }

  async function runPreflight() {
    if (!canExecute.value || busy.value) return;
    preflighting.value = true;
    operationError.value = '';
    preflightPassed.value = false;
    preflightMessage.value = '';
    let context;
    try {
      context = executionContext();
      await preflight(context);
    } catch (error) {
      operationError.value = failureText(error, '预检', { remote: context?.remote });
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
      operationError.value = failureText(error, '预检', { remote: context?.remote });
      diagnostics.value = safeText(error);
      return;
    } finally {
      preflighting.value = false;
    }
    const action = '完整版本部署';
    const serviceCount = bundlePlan.value?.services?.length || review.value?.services?.length || 0;
    const volumeNames = (bundlePlan.value?.dataVolumes || []).map((volume) => volume.name);
    const scope = `部署全部 ${serviceCount} 个服务，停止全部服务后备份数据卷并启动新版本；保留的数据卷：${volumeNames.join('、') || '无'}；失败时自动恢复上一版本`;
    executing.value = true;
    try {
      try {
        await ElMessageBox.confirm(
          `目标：${context.label}；目录：${context.installDir}；版本：${
            context.version || '未标记'
          }。${scope}。部署期间业务会短暂停机，失败恢复期间的新写入会撤回。是否继续？`,
          `确认${action}`,
          {
            type: 'warning',
            confirmButtonText: '开始部署',
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
      const response = await invokeVersionDeployment(context, false);
      diagnostics.value += `\n${safeText(response.output)}`;
      transaction.value = response.status || null;
      if (!response.success) {
        const reason = response.error
          ? failureText(response.error, '新版本部署', { remote: context.remote })
          : '请展开执行详情查看失败阶段和原因。';
        result.value = {
          success: false,
          recovered: Boolean(response.recovered),
          recoveryFailed: Boolean(response.recoveryFailed),
          backupDir: null,
          snapshotDir: response.snapshotDir || '',
          transactionId: response.transactionId || '',
          message: response.status?.deploymentStarted === false
            ? `本次部署在修改目标前停止，目标文件与数据未被本次部署修改。${reason}`
            : response.recovered
            ? `上一版本 ${response.previousVersion || '部署前版本'} 已恢复运行。${reason}`
            : response.recoveryFailed
            ? `自动恢复未完成，备份与事务现场已保留。${reason}`
            : reason
        };
        operationError.value = '';
        phase.value = '';
        return;
      }
      phase.value = '正在检查服务运行状态';
      const verified = await inspectDeploymentReview(
        context.bundleDir,
        context.installDir,
        contextTarget(context),
        context.imageDir
      );
      const unavailableFiles = (verified.artifacts || []).filter((artifact) =>
        ['missing', 'invalid', 'unconfirmed'].includes(artifact.status)
      );
      if (unavailableFiles.length) {
        throw new Error(
          `部署后的制品文件尚未就绪：${unavailableFiles
            .map((artifact) => artifact.label || artifact.key)
            .join('、')}。请返回清单比对检查。`
        );
      }
      const ready = servicesAreReady(verified);
      result.value = {
        success: ready,
        pending: !ready,
        recovered: false,
        recoveryFailed: false,
        snapshotDir: response.snapshotDir || '',
        transactionId: response.transactionId || '',
        message: ready
          ? '服务已运行，可前往集群监控检查业务状态。'
          : '执行命令已完成，部分服务的就绪状态仍需确认，请前往集群监控检查。'
      };
      phase.value = '';
    } catch (error) {
      result.value = {
        ...result.value,
        snapshotDir: error.result?.snapshotDir || result.value?.snapshotDir,
        transactionId: error.result?.transactionId || result.value?.transactionId,
        success: false,
        message: '执行未完成，请查看执行详情，处理后重新比对。'
      };
      operationError.value = failureText(error, action, { remote: context.remote });
      diagnostics.value += `\n${safeText(error.result?.output || '')}\n${safeText(error)}`;
      phase.value = '';
    } finally {
      executing.value = false;
      executionFeedback.stop();
    }
  }

  async function restorePreviousVersion() {
    if (busy.value || !config.installDir?.trim()) return;
    try {
      await ElMessageBox.confirm(
        `目标：${targetLabel.value}；目录：${config.installDir}。将从部署事务快照恢复上一版本和业务数据，恢复期间服务会停机。是否继续？`,
        '恢复上一版本',
        {
          type: 'warning',
          confirmButtonText: '开始恢复',
          cancelButtonText: '取消',
          closeOnClickModal: false
        }
      );
    } catch {
      return;
    }
    recoveringTransaction.value = true;
    operationError.value = '';
    try {
      const response = await recoverDeploymentTransaction(
        config.installDir.trim(),
        target()
      );
      diagnostics.value = safeText(response.output);
      transaction.value = response.status || null;
      result.value = {
        success: false,
        recovered: Boolean(response.recovered),
        recoveryFailed: Boolean(response.recoveryFailed),
        snapshotDir: response.snapshotDir || '',
        transactionId: response.transactionId || '',
        message: response.recovered
          ? `上一版本 ${response.previousVersion || '部署前版本'} 已恢复运行。`
          : response.status?.deploymentStarted === false
          ? '本次事务已结束，无需恢复服务、文件或数据卷。'
          : response.recoveryFailed
          ? '恢复未完成，备份与事务现场已保留，请展开执行详情并联系工程师处理。'
          : '首次部署的服务与数据卷已清理。'
      };
      if (response.error) operationError.value = safeText(response.error);
    } catch (error) {
      operationError.value = failureText(error, '恢复上一版本', {
        remote: isRemote.value
      });
      diagnostics.value = safeText(error);
    } finally {
      recoveringTransaction.value = false;
      await inspectTransaction();
    }
  }

  function executionContext() {
    return createDeploymentContext({
      connection: config,
      remote: isRemote.value,
      bundle: preparedBundle.value,
      label: targetLabel.value,
      imageDir: imageMediaDir.value
    });
  }

  const { progressStep, progressPercent, progressMessage, elapsedText } = executionFeedback;

  return { steps, activeStep, selectedHost, isRemote, targetLabel, hostReady, testingSsh, settingKey, sshMessage, sshOk, envData, envError, checkingEnv, installingDocker, artifactPicker, candidate, preparedBundle, artifactBusy, review, reviewError, checkingReview, reviewTab, imageMediaDir, imageSource, imagePullGroups, imagePending, imagePullable, pendingDigestImages, imageCounts, imageBlockedCount, imageNextBlockedReason, imageRegistryLoadError, imagePhase, imageProgress, imageDiagnostics, imageResult, importingImages, bundlePlan, transaction, recoveringTransaction, executing, preflighting, preflightPassed, preflightMessage, phase, operationError, diagnostics, result, busy, environmentReady, canExecute, artifactSelectionIssue, artifactChosen, artifactPrepared, testConnection, setupAccess, checkEnvironment, installDocker, nextStep, previousStep, loadReview, pickImageDirectory, prepareImages, setImagePullSelection, runPreflight, executeDeployment, restorePreviousVersion, progressStep, progressPercent, progressMessage, elapsedText };
}
