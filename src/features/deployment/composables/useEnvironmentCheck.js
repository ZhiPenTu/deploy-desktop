import { computed, ref } from 'vue';
import { ElMessageBox } from 'element-plus';
import {
  checkLocalEnv,
  checkRemoteEnv,
  installOfflineDockerEngine,
  installRemoteDockerEngine
} from '../../../api/tauri.js';
import { failureText } from '../domain/errors.js';

/**
 * 目标环境的检测与 Docker 安装。
 * 检测结果只反映一次请求，返回 invalidate/dispose 用于作废仍在途的旧结果。
 */
export function useEnvironmentCheck({ config, isRemote, targetLabel, desktopGuard, onInvalidateReview }) {
  const envData = ref(null);
  const envError = ref('');
  const checkingEnv = ref(false);
  const installingDocker = ref(false);

  let envRevision = 0;
  let disposed = false;

  const environmentReady = computed(() =>
    Boolean(envData.value?.docker_running && envData.value?.compose_installed && !envError.value)
  );

  async function checkEnvironment() {
    const current = ++envRevision;
    checkingEnv.value = true;
    envError.value = '';
    envData.value = null;
    onInvalidateReview?.();
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
      if (current === envRevision && !disposed) envError.value = failureText(error, '环境检测');
    } finally {
      if (current === envRevision && !disposed) checkingEnv.value = false;
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

  function invalidate() {
    envRevision += 1;
  }

  function dispose() {
    disposed = true;
    envRevision += 1;
  }

  return {
    envData,
    envError,
    checkingEnv,
    installingDocker,
    environmentReady,
    checkEnvironment,
    installDocker,
    invalidate,
    dispose
  };
}
