import { ref } from 'vue';
import { ElMessageBox } from 'element-plus';
import { setupManagedHostAccess, testManagedHostConnection } from '../../../api/tauri.js';
import { failureText } from '../domain/errors.js';

/** 目标主机的连通性检查与免密登录配置。 */
export function useHostAccess({ config, targetLabel, desktopGuard }) {
  const testingSsh = ref(false);
  const settingKey = ref(false);
  const sshMessage = ref('');
  const sshOk = ref(false);

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

  function reset() {
    sshMessage.value = '';
  }

  return { testingSsh, settingKey, sshMessage, sshOk, testConnection, setupAccess, reset };
}
