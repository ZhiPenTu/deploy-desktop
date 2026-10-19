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

      <label class="field-label" for="update-endpoint">更新源地址</label>
      <el-input
        id="update-endpoint"
        v-model="endpoint"
        placeholder="https://github.com/<owner>/<repo>/releases/latest/download/latest.json"
        clearable
        :disabled="busy"
        @keyup.enter="checkNow"
      />
      <el-checkbox v-model="allowInsecure" :disabled="busy" @change="saveSettings">
        允许 HTTP 更新源（内网镜像）
      </el-checkbox>
      <p class="field-hint">
        填写 latest.json 的完整地址。检查与下载均在本机完成，地址会保存在本机偏好设置中。内网镜像使用 HTTP 时才需要勾选上方选项。
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
import { computed, onMounted, ref } from 'vue';
import { Channel, invoke } from '@tauri-apps/api/core';
import { ElMessage } from 'element-plus';
import { Download } from '@element-plus/icons-vue';

const currentVersion = __APP_VERSION__;

const visible = ref(false);
const endpoint = ref('');
const allowInsecure = ref(false);
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

const saveSettings = async () => {
  try {
    await invoke('save_update_settings', {
      settings: { endpoint: endpoint.value.trim(), allowInsecure: allowInsecure.value },
    });
  } catch (error) {
    ElMessage.warning(`更新设置保存失败：${error}`);
  }
};

const checkNow = async () => {
  const url = endpoint.value.trim();
  if (!url) {
    ElMessage.warning('请填写更新源地址');
    return;
  }

  try {
    await invoke('save_update_settings', {
      settings: { endpoint: url, allowInsecure: allowInsecure.value },
    });
  } catch (error) {
    ElMessage.warning(`更新源地址保存失败：${error}`);
  }
  checking.value = true;
  status.value = '';
  errorMessage.value = '';
  updateInfo.value = null;
  downloadedBytes.value = 0;
  totalBytes.value = 0;

  try {
    const info = await invoke('check_app_update', {
      endpoint: url,
      allowInsecure: allowInsecure.value,
    });
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

  const channel = new Channel();
  channel.onmessage = (event) => {
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
    await invoke('install_app_update', { onEvent: channel });
    progressText.value = '安装完成，正在重启应用';
  } catch (error) {
    installing.value = false;
    status.value = 'error';
    errorMessage.value = String(error);
  }
};

onMounted(async () => {
  try {
    const settings = await invoke('get_update_settings');
    endpoint.value = settings.endpoint || '';
    allowInsecure.value = Boolean(settings.allowInsecure);
  } catch {
    endpoint.value = '';
  }
});
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
  background: rgba(255, 255, 255, 0.025);
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

.field-label {
  margin-top: 4px;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-secondary);
}

.field-hint {
  margin: -2px 0 0;
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-muted);
}

.update-card {
  padding: 12px 14px;
  border: 1px solid rgba(79, 140, 255, 0.28);
  border-radius: 10px;
  background: rgba(79, 140, 255, 0.08);
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
