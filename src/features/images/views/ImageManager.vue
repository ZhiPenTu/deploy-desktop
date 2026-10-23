<template>
  <div class="page-view image-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">镜像与离线包</h2>
        <p class="page-subtitle">管理本机 Docker 镜像，按需导出离线包并分发到目标机</p>
      </div>
      <div class="header-actions">
        <el-button :loading="refreshing" @click="refreshAll">
          <el-icon><Refresh /></el-icon>
          <span>刷新</span>
        </el-button>
      </div>
    </div>

    <el-tabs v-model="activeTab" class="image-tabs" @tab-change="handleTabChange">
      <el-tab-pane label="镜像列表" name="images">
        <ImageListPanel
          ref="imageListRef"
          :images="dockerImages"
          :loading="loadingImages"
          :error="imagesError"
          :selected-count="selectedImages.length"
          :busy="busyLocal"
          :exporting="exporting"
          :deleting="deletingImages"
          :offline-dir="offlineDir"
          @refresh="loadImages"
          @pull="openPullDialog"
          @export-selected="handleExport"
          @delete-selected="handleDeleteImages"
          @selection-change="handleImageSelection"
        />
      </el-tab-pane>

      <el-tab-pane label="离线包列表" name="packages">
        <OfflinePackagesPanel
          ref="packageListRef"
          :scan="offlineScan"
          :loading="loadingPackages"
          :error="packagesError"
          :selected-count="selectedPackages.length"
          :busy="busyLocal"
          :importing="importingPackages"
          :deleting="deletingPackages"
          :configured-dir="offlineDir"
          @refresh="loadPackages"
          @import-selected="handleImportPackages"
          @delete-selected="handleDeletePackages"
          @edit-directory="goSettings"
          @go-images="activeTab = 'images'"
          @selection-change="handlePackageSelection"
        />
      </el-tab-pane>

      <el-tab-pane v-if="remoteHosts.length" label="远程分发" name="remote">
        <RemoteImageDistribution
          :hosts="remoteHosts"
          :initial-host-id="projectHostId"
          :connection="connection"
        />
      </el-tab-pane>
    </el-tabs>

    <div v-if="progress.length" class="card-surface progress-card">
      <div class="progress-head">
        <span>执行进度</span>
        <span class="hint">{{ progressMessage }}</span>
      </div>
      <el-progress :percentage="currentPercent" />
      <div class="progress-list">
        <div
          v-for="(item, index) in progress.slice(-6)"
          :key="`${item.reference}-${index}`"
          class="progress-row"
        >
          <span class="progress-percent">{{ item.percent }}%</span>
          <span class="progress-text">{{ item.message }}</span>
        </div>
      </div>
    </div>

    <div v-if="resultLog" class="card-surface result-card">
      <div class="card-title-bar">
        <h3>执行结果</h3>
        <span class="hint">{{ resultTitle }}</span>
      </div>
      <pre class="output-log">{{ resultLog }}</pre>
    </div>

    <PullImageDialog
      v-model:visible="pullDialogVisible"
      :sources="registrySources"
      :loading-sources="loadingSources"
      :pulling="pulling"
      @submit="runPull"
      @go-settings="goSourceSettings"
    />
  </div>
</template>

<script setup>
import { useDesktopProgress } from '../../../shared/composables/useDesktopProgress.js';
import { computed, onMounted, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Refresh } from '@element-plus/icons-vue';
import ImageListPanel from '../components/ImageListPanel.vue';
import OfflinePackagesPanel from '../components/OfflinePackagesPanel.vue';
import PullImageDialog from '../components/PullImageDialog.vue';
import RemoteImageDistribution from '../components/RemoteImageDistribution.vue';
import {
  exportLocalDockerImages,
  getImageLibrarySettings,
  importOfflinePackages,
  listLocalDockerImages,
  listRegistrySources,
  onImageLibraryProgress,
  pullLocalDockerImage,
  removeLocalDockerImages,
  removeOfflinePackages,
  scanOfflinePackages
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';
import { formatImageId } from '../../../shared/format.js';
import { connectionInventory } from '../../connections/state.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const emit = defineEmits(['go-settings']);

const remoteHosts = computed(() =>
  (connectionInventory.hosts || []).filter((host) => host.kind !== 'local')
);
const projectHostId = computed(() => {
  const id = props.connection.mode === 'remote' ? props.connection.activeHostId : '';
  return remoteHosts.value.some((host) => host.id === id) ? id : '';
});
const activeTab = ref('images');
const dockerImages = ref([]);
const offlineScan = ref(null);
const offlineDir = ref('');
const imagesError = ref('');
const packagesError = ref('');
const loadingImages = ref(false);
const loadingPackages = ref(false);
const loadingSettings = ref(false);
const exporting = ref(false);
const deletingImages = ref(false);
const deletingPackages = ref(false);
const importingPackages = ref(false);
const pulling = ref(false);
const imageSelection = ref([]);
const packageSelection = ref([]);
const progress = ref([]);
const resultLog = ref('');
const resultTitle = ref('');
const pullDialogVisible = ref(false);
const imageListRef = ref(null);
const packageListRef = ref(null);
const registrySources = ref([]);
const loadingSources = ref(false);
function goSourceSettings() {
  pullDialogVisible.value = false;
  emit('go-settings', 'images');
}

const refreshing = computed(
  () => loadingImages.value || loadingPackages.value || loadingSettings.value
);
const busyLocal = computed(
  () =>
    exporting.value ||
    deletingImages.value ||
    deletingPackages.value ||
    importingPackages.value ||
    pulling.value
);
const selectedImages = computed(() => imageSelection.value);
const selectedPackages = computed(() => packageSelection.value);
const currentPercent = computed(() => {
  if (!progress.value.length) {
    return 0;
  }
  return progress.value[progress.value.length - 1].percent;
});
const progressMessage = computed(() => {
  if (!progress.value.length) {
    return '';
  }
  const last = progress.value[progress.value.length - 1];
  return `${last.current} / ${last.total} · ${last.reference}`;
});

const imageLabel = (image) =>
  image.exportable ? image.reference : `未标记镜像（${formatImageId(image.id)}）`;

const loadSettings = async () => {
  loadingSettings.value = true;
  try {
    const settings = await getImageLibrarySettings();
    offlineDir.value = settings.offlineDir || '';
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    loadingSettings.value = false;
  }
};

const loadImages = async () => {
  loadingImages.value = true;
  imagesError.value = '';
  try {
    dockerImages.value = await listLocalDockerImages();
    imageSelection.value = [];
    imageListRef.value?.clearSelection?.();
  } catch (error) {
    dockerImages.value = [];
    imageSelection.value = [];
    imagesError.value = errorText(error);
  } finally {
    loadingImages.value = false;
  }
};

const loadPackages = async () => {
  loadingPackages.value = true;
  packagesError.value = '';
  try {
    offlineScan.value = await scanOfflinePackages();
    if (offlineScan.value?.offlineDir) {
      offlineDir.value = offlineScan.value.offlineDir;
    }
    packageSelection.value = [];
    packageListRef.value?.clearSelection?.();
  } catch (error) {
    offlineScan.value = null;
    packageSelection.value = [];
    packagesError.value = errorText(error);
  } finally {
    loadingPackages.value = false;
  }
};

const refreshAll = async () => {
  await Promise.all([loadSettings(), loadImages(), loadPackages()]);
};

const handleTabChange = (name) => {
  if (name === 'images') {
    loadImages();
  } else if (name === 'packages') {
    loadPackages();
  }
};

const handleImageSelection = (rows) => {
  imageSelection.value = rows;
};

const handlePackageSelection = (rows) => {
  packageSelection.value = rows;
};

const openPullDialog = async () => {
  pullDialogVisible.value = true;
  loadingSources.value = true;
  try {
    registrySources.value = await listRegistrySources();
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    loadingSources.value = false;
  }
};

const runPull = async (payload) => {
  pulling.value = true;
  progress.value = [];
  resultLog.value = '';
  try {
    const output = await pullLocalDockerImage(
      payload.reference,
      payload.username,
      payload.password,
      payload.registrySourceId
    );
    resultLog.value = output;
    resultTitle.value = `已拉取 ${payload.resolved}`;
    ElMessage.success('镜像拉取完成');
    pullDialogVisible.value = false;
    await loadImages();
  } catch (error) {
    resultLog.value = errorText(error);
    resultTitle.value = '镜像拉取失败';
    ElMessage.error('镜像拉取失败');
  } finally {
    pulling.value = false;
  }
};

const handleExport = async () => {
  if (!selectedImages.value.length) {
    return;
  }
  const blocked = selectedImages.value.filter((image) => !image.exportable);
  if (blocked.length) {
    ElMessage.warning('未标记镜像无法导出，请先为镜像添加标签');
    return;
  }
  const references = selectedImages.value.map((image) => image.reference);
  exporting.value = true;
  progress.value = [];
  resultLog.value = '';
  try {
    const results = await exportLocalDockerImages(references);
    const succeeded = results.filter((item) => item.success);
    const failed = results.filter((item) => !item.success);
    resultLog.value = results
      .map((item) => `${item.success ? '[成功]' : '[失败]'} ${item.reference}：${item.message}`)
      .join('\n');
    resultTitle.value = `导出完成：成功 ${succeeded.length} 个，失败 ${failed.length} 个`;
    if (succeeded.length) {
      ElMessage.success(`已导出 ${succeeded.length} 个离线包`);
      await loadPackages();
      activeTab.value = 'packages';
    }
    if (failed.length) {
      ElMessage.warning(`有 ${failed.length} 个镜像导出失败，请查看执行结果`);
    }
  } catch (error) {
    resultLog.value = errorText(error);
    resultTitle.value = '导出失败';
    ElMessage.error('导出离线包失败');
  } finally {
    exporting.value = false;
  }
};

const handleDeleteImages = async () => {
  if (!selectedImages.value.length) {
    return;
  }
  const images = [...selectedImages.value];
  const names = images.slice(0, 5).map(imageLabel).join('、');
  const more = images.length > 5 ? ` 等 ${images.length} 个镜像` : '';
  try {
    await ElMessageBox.confirm(
      `将从本机 Docker 删除以下镜像：${names}${more}。\n\n如果镜像仍被容器使用，删除会失败。此操作不可撤销，是否继续？`,
      '确认删除镜像',
      { confirmButtonText: '删除', cancelButtonText: '取消', type: 'warning' }
    );
  } catch {
    return;
  }

  const references = images.map((image) => image.reference);
  deletingImages.value = true;
  progress.value = [];
  resultLog.value = '';
  try {
    const results = await removeLocalDockerImages(references);
    const succeeded = results.filter((item) => item.success);
    const failed = results.filter((item) => !item.success);
    resultLog.value = results
      .map((item) => `${item.success ? '[成功]' : '[失败]'} ${item.target}：${item.message}`)
      .join('\n');
    resultTitle.value = `删除完成：成功 ${succeeded.length} 个，失败 ${failed.length} 个`;
    if (failed.length) {
      ElMessage.warning(`有 ${failed.length} 个镜像未能删除，请查看执行结果`);
    } else {
      ElMessage.success(`已删除 ${succeeded.length} 个镜像`);
    }
    await loadImages();
  } catch (error) {
    resultLog.value = errorText(error);
    resultTitle.value = '删除镜像失败';
    ElMessage.error('删除镜像失败');
  } finally {
    deletingImages.value = false;
  }
};

const handleDeletePackages = async () => {
  if (!selectedPackages.value.length) {
    return;
  }
  const files = selectedPackages.value.map((file) => file.name);
  const names = files.slice(0, 6).join('、');
  const more = files.length > 6 ? ` 等 ${files.length} 个文件` : '';
  try {
    await ElMessageBox.confirm(
      `将从 ${offlineScan.value?.offlineDir || offlineDir.value} 删除以下离线包：${names}${more}。\n\n只删除离线包文件，不影响本机 Docker 镜像。是否继续？`,
      '确认删除离线包',
      { confirmButtonText: '删除', cancelButtonText: '取消', type: 'warning' }
    );
  } catch {
    return;
  }

  deletingPackages.value = true;
  progress.value = [];
  resultLog.value = '';
  try {
    const results = await removeOfflinePackages(files);
    const succeeded = results.filter((item) => item.success);
    const failed = results.filter((item) => !item.success);
    resultLog.value = results
      .map((item) => `${item.success ? '[成功]' : '[失败]'} ${item.target}：${item.message}`)
      .join('\n');
    resultTitle.value = `删除完成：成功 ${succeeded.length} 个，失败 ${failed.length} 个`;
    if (failed.length) {
      ElMessage.warning(`有 ${failed.length} 个离线包未能删除，请查看执行结果`);
    } else {
      ElMessage.success(`已删除 ${succeeded.length} 个离线包`);
    }
    await loadPackages();
  } catch (error) {
    resultLog.value = errorText(error);
    resultTitle.value = '删除离线包失败';
    ElMessage.error('删除离线包失败');
  } finally {
    deletingPackages.value = false;
  }
};

const handleImportPackages = async () => {
  if (!selectedPackages.value.length) {
    return;
  }
  const files = selectedPackages.value.map((file) => file.name);
  const names = files.slice(0, 6).join('、');
  const more = files.length > 6 ? ` 等 ${files.length} 个文件` : '';
  try {
    await ElMessageBox.confirm(
      `将把以下离线包导入本机 Docker：${names}${more}。\n\n导入只读取离线包，不会删除文件。是否继续？`,
      '确认导入镜像',
      { confirmButtonText: '导入', cancelButtonText: '取消', type: 'info' }
    );
  } catch {
    return;
  }

  importingPackages.value = true;
  progress.value = [];
  resultLog.value = '';
  try {
    const results = await importOfflinePackages(files);
    const succeeded = results.filter((item) => item.success);
    const failed = results.filter((item) => !item.success);
    resultLog.value = results
      .map((item) => `${item.success ? '[成功]' : '[失败]'} ${item.target}：${item.message}`)
      .join('\n');
    resultTitle.value = `导入完成：成功 ${succeeded.length} 个，失败 ${failed.length} 个`;
    if (failed.length) {
      ElMessage.warning(`有 ${failed.length} 个离线包未能导入，请查看执行结果`);
    } else {
      ElMessage.success(`已导入 ${succeeded.length} 个离线包`);
    }
    await loadImages();
  } catch (error) {
    resultLog.value = errorText(error);
    resultTitle.value = '导入离线包失败';
    ElMessage.error('导入离线包失败');
  } finally {
    importingPackages.value = false;
  }
};

const goSettings = () => {
  emit('go-settings', 'images');
};

useDesktopProgress(onImageLibraryProgress, (payload) => {
  progress.value.push(payload);
}, () => ElMessage.warning('进度暂不可用，请等待执行结果。'));

onMounted(async () => {
  await refreshAll();
});
</script>

<style scoped>
.image-tabs {
  height: 100%;
}

.image-tabs :deep(.el-tabs__header) {
  margin-bottom: 18px;
}

.image-tabs :deep(.el-tabs__content) {
  overflow: visible;
}

.result-card {
  padding: 20px;
  margin-bottom: 18px;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.card-title-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 14px;
  margin-bottom: 16px;
}

.card-title-bar h3 {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-main);
}

.card-title-bar .hint {
  font-size: 11px;
  color: var(--text-muted);
}

.progress-card {
  padding: 20px;
  margin-bottom: 18px;
}

.progress-head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  gap: 12px;
  margin-bottom: 10px;
  font-size: 12px;
  color: var(--text-secondary);
}

.progress-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 10px;
}

.progress-row {
  display: flex;
  gap: 10px;
  align-items: baseline;
  font-size: 11px;
  color: var(--text-muted);
}

.progress-percent {
  width: 42px;
  text-align: right;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  color: var(--text-secondary);
}

.progress-text {
  overflow-wrap: anywhere;
}

.output-log {
  max-height: 260px;
  overflow: auto;
  margin: 0;
  padding: 14px;
  border-radius: var(--radius-sm);
  background: #0b1120;
  border: 1px solid #1e293b;
  color: #cbd5e1;
  font-size: 12px;
  line-height: 1.6;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: pre-wrap;
}
</style>
