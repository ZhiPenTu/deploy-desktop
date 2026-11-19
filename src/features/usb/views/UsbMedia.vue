<template>
  <div class="page-view usb-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">USB 中转</h2>
        <p class="page-subtitle">将交付包与离线镜像包写入 USB 介质，或从介质读取后用于部署更新</p>
      </div>
      <div class="header-actions">
        <el-button :loading="refreshing" :disabled="transferBusy" @click="refreshAll">
          <el-icon><Refresh /></el-icon>
          <span>刷新</span>
        </el-button>
        <el-button type="primary" :disabled="!desktopAvailable || transferBusy" @click="chooseMediaDir">
          选择介质目录
        </el-button>
      </div>
    </div>

    <el-alert
      v-if="!desktopAvailable"
      title="暂不可用"
      description="请在桌面应用中选择 USB 目录并写入或读取介质内容。"
      type="info"
      :closable="false"
      show-icon
      class="page-alert"
    />

    <div v-if="!mediaDir" class="card-surface usb-intro">
      <div class="empty-state">
        <span class="empty-title">尚未选择 USB 介质目录</span>
        <span class="empty-hint">
          将 U 盘或移动硬盘连接到电脑，选择一个目录作为中转介质。写入后可在另一台电脑上读取交付包与镜像包。
        </span>
        <el-button type="primary" :disabled="!desktopAvailable" @click="chooseMediaDir">
          选择介质目录
        </el-button>
      </div>
    </div>

    <template v-else>
      <el-alert
        v-if="scanError"
        :title="scanError"
        type="error"
        :closable="false"
        show-icon
        class="page-alert"
      >
        <el-button size="small" @click="loadScan">重新读取</el-button>
      </el-alert>

      <div class="card-surface media-summary">
        <div class="summary-main">
          <span class="summary-label">介质目录</span>
          <code class="path-text">{{ mediaRootPath }}</code>
          <el-tag v-if="scan && !scan.writable" size="small" type="danger" effect="plain">
            不可写入
          </el-tag>
        </div>
        <div class="summary-meta">
          <span>交付包 {{ artifactCount }} 个</span>
          <span>镜像包 {{ imageCount }} 个</span>
          <span>合计 {{ formatSize(scan?.totalBytes || 0) }}</span>
        </div>
        <p v-if="scan && !scan.exists" class="summary-hint">
          该目录还没有介质内容，首次写入时会创建「部署舱介质」文件夹。
        </p>
      </div>

      <el-tabs v-model="activeTab" class="usb-tabs">
        <el-tab-pane label="写入 USB" name="write">
          <UsbWritePanel
            :media-dir="mediaDir"
            :scan="scan"
            :local-bundles="localBundles"
            :bundles-loading="bundlesLoading"
            :bundles-error="bundlesError"
            :offline-scan="offlineScan"
            :packages-loading="packagesLoading"
            :packages-error="packagesError"
            :desktop-available="desktopAvailable"
            @refresh-bundles="loadBundles"
            @refresh-packages="loadPackages"
            @exported="handleExported"
            @go-images="emit('go-images')"
            @busy-change="transferBusy = $event"
          />
        </el-tab-pane>
        <el-tab-pane label="从 USB 读取" name="read">
          <UsbReadPanel
            :entries="scan?.entries || []"
            :loading="scanning"
            :error="scanError"
            :media-dir="mediaDir"
            :desktop-available="desktopAvailable"
            @refresh="loadScan"
            @deploy="handleDeploy"
            @imported="handleImported"
            @busy-change="transferBusy = $event"
          />
        </el-tab-pane>
      </el-tabs>
    </template>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { ElMessage } from 'element-plus';
import { Refresh } from '@element-plus/icons-vue';
import {
  isDesktopApp,
  listLocalArtifactBundles,
  scanOfflinePackages,
  scanUsbMedia,
  selectLocalPath
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';
import { formatSize } from '../../../shared/format.js';
import UsbWritePanel from '../components/UsbWritePanel.vue';
import UsbReadPanel from '../components/UsbReadPanel.vue';

const emit = defineEmits(['go-wizard', 'go-images']);

const desktopAvailable = isDesktopApp();
const mediaDir = ref('');
const activeTab = ref('write');
const scan = ref(null);
const scanError = ref('');
const scanning = ref(false);
const localBundles = ref([]);
const bundlesLoading = ref(false);
const bundlesError = ref('');
const offlineScan = ref(null);
const packagesLoading = ref(false);
const packagesError = ref('');
const transferBusy = ref(false);

const refreshing = computed(
  () => scanning.value || bundlesLoading.value || packagesLoading.value
);
const artifactCount = computed(
  () => (scan.value?.entries || []).filter((entry) => entry.kind === 'artifact').length
);
const imageCount = computed(
  () => (scan.value?.entries || []).filter((entry) => entry.kind === 'image').length
);
const mediaRootPath = computed(() => scan.value?.rootDir || mediaDir.value);

async function loadBundles() {
  bundlesLoading.value = true;
  bundlesError.value = '';
  try {
    localBundles.value = await listLocalArtifactBundles();
  } catch (readError) {
    bundlesError.value = errorText(readError, '读取本机交付包失败，请重试。');
  } finally {
    bundlesLoading.value = false;
  }
}

async function loadPackages() {
  packagesLoading.value = true;
  packagesError.value = '';
  try {
    offlineScan.value = await scanOfflinePackages();
  } catch (readError) {
    packagesError.value = errorText(readError, '读取本机离线镜像包失败，请重试。');
  } finally {
    packagesLoading.value = false;
  }
}

let scanRequest = 0;

async function loadScan() {
  if (!desktopAvailable) return;
  if (!mediaDir.value) {
    scan.value = null;
    return;
  }
  const request = ++scanRequest;
  scanning.value = true;
  scanError.value = '';
  try {
    const result = await scanUsbMedia(mediaDir.value);
    if (request !== scanRequest) return;
    scan.value = result;
  } catch (readError) {
    if (request !== scanRequest) return;
    scan.value = null;
    scanError.value = errorText(readError, '读取 USB 介质失败，请重新选择目录。');
  } finally {
    if (request === scanRequest) scanning.value = false;
  }
}

async function refreshAll() {
  if (!desktopAvailable) return;
  await Promise.all([loadBundles(), loadPackages(), loadScan()]);
}

async function chooseMediaDir() {
  if (!desktopAvailable || transferBusy.value) return;
  try {
    const selected = await selectLocalPath({
      directory: true,
      multiple: false,
      title: '选择 USB 介质目录'
    });
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) return;
    mediaDir.value = path;
    activeTab.value = 'write';
    await loadScan();
  } catch {
    ElMessage.error('无法选择目录，请重试。');
  }
}

function handleExported(result) {
  if (result?.scan) scan.value = result.scan;
}

function handleImported() {
  loadPackages();
}

function handleDeploy(preset) {
  emit('go-wizard', preset);
}

onMounted(refreshAll);
</script>

<style scoped>
.usb-view {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.usb-view .page-header {
  margin-bottom: 4px;
}

.page-alert {
  margin-bottom: 0;
}

.usb-intro {
  padding: 26px;
}

.usb-intro .empty-state {
  border: none;
  background: transparent;
  min-height: 160px;
}

.media-summary {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 16px 20px;
}

.summary-main {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.summary-label {
  font-size: 12px;
  color: var(--text-muted);
}

.summary-meta {
  display: flex;
  align-items: center;
  gap: 18px;
  flex-wrap: wrap;
  font-size: 12px;
  color: var(--text-secondary);
}

.summary-hint {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-muted);
}

.usb-tabs :deep(.el-tabs__header) {
  margin-bottom: 16px;
}

@media (max-width: 760px) {
  .usb-intro {
    padding: 16px;
  }
}
</style>
