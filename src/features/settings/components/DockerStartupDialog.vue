<template>
  <el-dialog
    v-model="visible"
    :title="installation ? '未检测到 Docker' : '无法检查 Docker'"
    width="min(520px, calc(100vw - 32px))"
    append-to-body
    :close-on-click-modal="false"
    :close-on-press-escape="!busy"
    :show-close="!busy"
  >
    <div class="docker-setup-body">
      <template v-if="installation">
        <el-alert
          :title="`当前${platformLabel}电脑尚未安装 Docker`"
          type="warning"
          :closable="false"
          show-icon
        />
        <p>本机部署和镜像操作需要 Docker。你可以稍后安装，继续管理远程主机。</p>
        <p>点击“安装 Docker”将打开官方安装页面，请按说明完成安装，再返回重新检测。</p>
      </template>
      <p v-else>暂时无法确认本机是否安装了 Docker，请重新检测。你也可以稍后处理，继续管理远程主机。</p>
      <el-alert
        v-if="errorMessage"
        :title="errorMessage"
        type="error"
        :closable="false"
        show-icon
      />
      <el-alert
        v-else-if="installationOpened"
        title="已请求打开 Docker 官方安装页面。完成安装后，请点击“重新检测”。"
        type="info"
        :closable="false"
        show-icon
      />
    </div>
    <template #footer>
      <div class="docker-setup-actions">
        <el-button :disabled="busy" @click="visible = false">稍后处理</el-button>
        <el-button :loading="checking" :disabled="opening" @click="checkInstallation(true)">
          重新检测
        </el-button>
        <el-button
          v-if="installation"
          type="primary"
          :loading="opening"
          :disabled="checking"
          @click="openInstallation"
        >
          安装 Docker
        </el-button>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { ElMessage } from 'element-plus';
import { checkDockerInstallation, isDesktopApp, openDockerInstallation } from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';

const visible = ref(false);
const checking = ref(false);
const opening = ref(false);
const installation = ref(null);
const installationOpened = ref(false);
const errorMessage = ref('');
const busy = computed(() => checking.value || opening.value);
const platformLabel = computed(() => {
  const label = { windows: 'Windows', linux: 'Linux', macos: 'macOS' }[installation.value?.os];
  return label ? ` ${label} ` : '';
});
let disposed = false;

async function checkInstallation(notify = false) {
  if (busy.value || !isDesktopApp()) return;
  checking.value = true;
  errorMessage.value = '';
  try {
    const result = await checkDockerInstallation();
    if (disposed) return;
    if (!result || typeof result.installed !== 'boolean') {
      throw new Error('未能获取 Docker 安装状态，请重新检测。');
    }
    installation.value = result;
    visible.value = !result.installed;
    if (result.installed && notify) ElMessage.success('已检测到 Docker。');
  } catch (error) {
    if (disposed) return;
    installation.value = null;
    errorMessage.value = errorText(error, '无法检查 Docker，请重新检测。');
    visible.value = true;
  } finally {
    if (!disposed) checking.value = false;
  }
}

async function openInstallation() {
  if (busy.value) return;
  opening.value = true;
  errorMessage.value = '';
  try {
    await openDockerInstallation();
    if (!disposed) installationOpened.value = true;
  } catch (error) {
    if (!disposed) errorMessage.value = errorText(error, '无法打开安装页面，请稍后重试。');
  } finally {
    if (!disposed) opening.value = false;
  }
}

function recheckOnFocus() {
  if (visible.value && installationOpened.value) checkInstallation(true);
}

onMounted(() => {
  if (!isDesktopApp()) return;
  window.addEventListener('focus', recheckOnFocus);
  checkInstallation();
});

onUnmounted(() => {
  disposed = true;
  window.removeEventListener('focus', recheckOnFocus);
});
</script>

<style scoped>
.docker-setup-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.docker-setup-body p {
  margin: 0;
  color: var(--text-secondary);
  line-height: 1.7;
}

.docker-setup-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
}

.docker-setup-actions .el-button + .el-button {
  margin-left: 0;
}
</style>
