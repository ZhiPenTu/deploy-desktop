<template>
  <el-button plain :loading="checking" @click="openDialog">
    <el-icon><Download /></el-icon>
    <span>检查更新</span>
  </el-button>

  <el-dialog
    v-model="visible"
    title="检查更新"
    width="540px"
    :close-on-click-modal="!installing"
    :close-on-press-escape="!installing"
    :show-close="!installing"
    @closed="resetState"
  >
    <div class="update-body">
      <div class="version-row">
        <span class="version-label">当前版本</span>
        <span class="version-value">{{ currentVersion }}</span>
      </div>

      <div class="version-row">
        <span class="version-label">更新来源</span>
        <span class="source-value">GitHub Releases</span>
      </div>
      <p class="field-hint">
        自动从 GitHub Releases 获取最新版本，检查与下载均在本机完成，无需手动配置更新源。
      </p>

      <el-alert
        v-if="status === 'latest'"
        title="当前已是最新版本"
        type="success"
        :closable="false"
        show-icon
      />
      <el-alert
        v-if="status === 'error'"
        :title="errorMessage"
        type="error"
        :closable="false"
        show-icon
      />

      <div v-if="updateInfo" class="update-card">
        <div class="update-head">
          <span class="update-version">发现新版本 {{ updateInfo.version }}</span>
          <span v-if="updateInfo.date" class="update-date">{{ formatDate(updateInfo.date) }}</span>
        </div>
        <pre v-if="updateInfo.body" class="update-notes">{{ updateInfo.body }}</pre>
      </div>

      <div v-if="installing" class="progress-block">
        <el-progress
          :percentage="percent"
          :status="percent >= 100 ? 'success' : ''"
          :stroke-width="8"
        />
        <span class="progress-text">{{ progressText }}</span>
      </div>
    </div>

    <template #footer>
      <el-button :disabled="installing" @click="visible = false">关闭</el-button>
      <el-button
        v-if="!updateInfo"
        type="primary"
        :loading="checking"
        :disabled="installing"
        @click="checkNow"
      >
        检查更新
      </el-button>
      <el-button
        v-else
        type="primary"
        :loading="installing"
        :disabled="checking"
        @click="installNow"
      >
        下载并安装
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, ref } from 'vue';
import { checkAppUpdate, installAppUpdate } from '../api/tauri.js';
import { Download } from '@element-plus/icons-vue';

const currentVersion = __APP_VERSION__;

const visible = ref(false);
const checking = ref(false);
const installing = ref(false);
const updateInfo = ref(null);
const status = ref('');
const errorMessage = ref('');
const downloadedBytes = ref(0);
const totalBytes = ref(0);
const progressText = ref('');

const busy = computed(() => checking.value || installing.value);
const percent = computed(() => {
  if (!totalBytes.value) {
    return installing.value ? 1 : 0;
  }
  return Math.min(100, Math.round((downloadedBytes.value / totalBytes.value) * 100));
});

const formatSize = (bytes) => {
  if (!bytes) {
    return '0 MB';
  }
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
};

const formatDate = (value) => {
  if (typeof value === 'number') {
    return new Date(value * 1000).toLocaleString();
  }
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
};

const openDialog = () => {
  visible.value = true;
  checkNow();
};

const resetState = () => {
  if (busy.value) {
    return;
  }
  status.value = '';
  errorMessage.value = '';
  updateInfo.value = null;
  downloadedBytes.value = 0;
  totalBytes.value = 0;
  progressText.value = '';
};

const checkNow = async () => {
  if (busy.value) {
    return;
  }
  checking.value = true;
  status.value = '';
  errorMessage.value = '';
  updateInfo.value = null;
  downloadedBytes.value = 0;
  totalBytes.value = 0;

  try {
    const info = await checkAppUpdate();
    if (info) {
      updateInfo.value = info;
      status.value = 'available';
    } else {
      status.value = 'latest';
    }
  } catch (error) {
    status.value = 'error';
    errorMessage.value = String(error);
  } finally {
    checking.value = false;
  }
};

const installNow = async () => {
  installing.value = true;
  status.value = '';
  errorMessage.value = '';
  downloadedBytes.value = 0;
  totalBytes.value = 0;
  progressText.value = '正在准备下载';

  const onProgress = (event) => {
    if (event.event === 'Started') {
      totalBytes.value = event.data.contentLength || 0;
      progressText.value = totalBytes.value
        ? `正在下载，共 ${formatSize(totalBytes.value)}`
        : '正在下载更新';
    } else if (event.event === 'Progress') {
      downloadedBytes.value += event.data.chunkLength;
      progressText.value = totalBytes.value
        ? `正在下载 ${formatSize(downloadedBytes.value)} / ${formatSize(totalBytes.value)}`
        : `已下载 ${formatSize(downloadedBytes.value)}`;
    } else if (event.event === 'Finished') {
      progressText.value = '下载完成，正在安装并在安装后重启应用';
    }
  };

  try {
    await installAppUpdate(onProgress);
    progressText.value = '安装完成，正在重启应用';
  } catch (error) {
    installing.value = false;
    status.value = 'error';
    errorMessage.value = String(error);
  }
};
</script>

<style scoped>
.update-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.version-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--panel-subtle);
}

.version-label {
  font-size: 12px;
  color: var(--text-muted);
}

.version-value {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
}

.source-value {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
}

.field-hint {
  margin: -2px 0 0;
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-muted);
}

.update-card {
  padding: 12px 14px;
  border: 1px solid #c3d3fa;
  border-radius: 10px;
  background: var(--primary-soft);
}

.update-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.update-version {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
}

.update-date {
  font-size: 11px;
  color: var(--text-muted);
}

.update-notes {
  margin: 10px 0 0;
  max-height: 180px;
  overflow: auto;
  font-family: inherit;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
}

.progress-block {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 4px;
}

.progress-text {
  font-size: 12px;
  color: var(--text-muted);
}
</style>
