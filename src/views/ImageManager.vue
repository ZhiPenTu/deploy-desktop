<template>
  <div class="page-view image-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">镜像与离线包</h2>
        <p class="page-subtitle">管理本机 Docker 镜像，按需导出离线包并分发到目标机</p>
      </div>
      <div class="header-actions">
        <el-tag v-if="isRemote" type="primary" effect="plain">
          远程 {{ connection.remoteHost }}
        </el-tag>
        <el-tag v-else type="info" effect="plain">本地部署</el-tag>
        <el-button :loading="refreshing" @click="refreshAll">
          <el-icon><Refresh /></el-icon>
          <span>刷新</span>
        </el-button>
      </div>
    </div>

    <el-tabs v-model="activeTab" class="image-tabs" @tab-change="handleTabChange">
      <el-tab-pane label="镜像列表" name="images">
        <div class="card-surface image-card">
          <div class="card-title-bar">
            <h3>本机 Docker 镜像</h3>
            <span class="hint">{{ imageSummary }}</span>
          </div>

          <div class="toolbar-row">
            <el-button :loading="loadingImages" @click="loadImages">
              <el-icon><Refresh /></el-icon>
              <span>刷新镜像</span>
            </el-button>
            <el-button type="primary" @click="openPullDialog">
              <el-icon><Download /></el-icon>
              <span>拉取镜像</span>
            </el-button>
          </div>

          <div class="selection-row">
            <span class="selection-hint">已选择 {{ selectedImages.length }} 个镜像</span>
            <el-button
              type="primary"
              plain
              :disabled="!selectedImages.length || busyLocal"
              :loading="exporting"
              @click="handleExport"
            >
              导出离线包
            </el-button>
            <el-button
              type="danger"
              plain
              :disabled="!selectedImages.length || busyLocal"
              :loading="deletingImages"
              @click="handleDeleteImages"
            >
              删除选中镜像
            </el-button>
            <span class="field-hint inline-hint">离线包保存到 {{ offlineDir || '设置中的目录' }}</span>
          </div>

          <el-alert
            v-if="imagesError"
            type="warning"
            :closable="false"
            show-icon
            class="error-alert"
          >
            <template #title>
              <span>{{ imagesError }}</span>
            </template>
            <div class="error-actions">
              <span>请确认 Docker 已安装并启动。</span>
              <el-button size="small" @click="loadImages">重新读取</el-button>
            </div>
          </el-alert>

          <el-table
            v-if="dockerImages.length"
            ref="imageTable"
            :data="dockerImages"
            row-key="reference"
            size="small"
            class="image-table"
            @selection-change="handleImageSelection"
          >
            <el-table-column type="selection" width="44" />
            <el-table-column label="镜像" min-width="300">
              <template #default="{ row }">
                <code v-if="row.exportable">{{ row.reference }}</code>
                <span v-else class="muted-text">未标记镜像</span>
              </template>
            </el-table-column>
            <el-table-column label="镜像 ID" width="140">
              <template #default="{ row }">
                <code class="muted-code">{{ shortId(row.id) }}</code>
              </template>
            </el-table-column>
            <el-table-column label="大小" width="110">
              <template #default="{ row }">{{ row.sizeText || formatSize(row.sizeBytes) }}</template>
            </el-table-column>
            <el-table-column label="创建时间" width="190">
              <template #default="{ row }">{{ row.createdText || '-' }}</template>
            </el-table-column>
            <el-table-column label="状态" width="110">
              <template #default="{ row }">
                <el-tag :type="row.exportable ? 'success' : 'info'" size="small" effect="plain">
                  {{ row.exportable ? '可导出' : '仅可删除' }}
                </el-tag>
              </template>
            </el-table-column>
          </el-table>

          <div v-else-if="!imagesError" class="empty-state">
            <span class="empty-title">{{ loadingImages ? '正在读取本机镜像' : '本机还没有 Docker 镜像' }}</span>
            <span class="empty-hint">
              {{ loadingImages ? '请稍候' : '填写镜像地址后拉取，或先在目标机上导入离线包。' }}
            </span>
            <div v-if="!loadingImages" class="empty-actions">
              <el-button size="small" type="primary" @click="openPullDialog">拉取镜像</el-button>
            </div>
          </div>

        </div>
      </el-tab-pane>

      <el-tab-pane label="离线包列表" name="packages">
        <div class="card-surface image-card">
          <div class="card-title-bar">
            <h3>离线镜像包</h3>
            <span class="hint">{{ packageSummary }}</span>
          </div>

          <div class="dir-row">
            <span class="dir-label">保存目录</span>
            <code class="path-text">{{ offlineScan?.offlineDir || offlineDir || '未配置' }}</code>
            <el-button size="small" @click="goSettings">修改目录</el-button>
          </div>

          <div class="selection-row">
            <span class="selection-hint">已选择 {{ selectedPackages.length }} 个离线包</span>
            <el-button
              type="primary"
              plain
              :disabled="!selectedPackages.length || busyLocal"
              :loading="importingPackages"
              @click="handleImportPackages"
            >
              导入到本机 Docker
            </el-button>
            <el-button
              type="danger"
              plain
              :disabled="!selectedPackages.length || busyLocal"
              :loading="deletingPackages"
              @click="handleDeletePackages"
            >
              删除选中离线包
            </el-button>
            <el-button :loading="loadingPackages" @click="loadPackages">
              <el-icon><Refresh /></el-icon>
              <span>刷新离线包</span>
            </el-button>
            <el-button @click="activeTab = 'images'">从镜像列表导出</el-button>
          </div>

          <el-alert
            v-if="packagesError"
            type="warning"
            :closable="false"
            show-icon
            :title="packagesError"
            class="error-alert"
          />

          <el-table
            v-if="offlinePackages.length"
            ref="packageTable"
            :data="offlinePackages"
            row-key="name"
            size="small"
            class="image-table"
            @selection-change="handlePackageSelection"
          >
            <el-table-column type="selection" width="44" />
            <el-table-column prop="name" label="离线包文件" min-width="260" />
            <el-table-column label="对应镜像" min-width="300">
              <template #default="{ row }">
                <template v-if="row.imageRefs && row.imageRefs.length">
                  <code v-for="image in row.imageRefs" :key="image" class="ref-line">{{ image }}</code>
                </template>
                <span v-else class="muted-text">未记录对应镜像</span>
              </template>
            </el-table-column>
            <el-table-column label="大小" width="110">
              <template #default="{ row }">{{ formatSize(row.sizeBytes) }}</template>
            </el-table-column>
            <el-table-column label="修改时间" width="180">
              <template #default="{ row }">{{ formatTime(row.modifiedMs) }}</template>
            </el-table-column>
          </el-table>

          <div v-else-if="!packagesError" class="empty-state">
            <span class="empty-title">
              {{ loadingPackages ? '正在读取离线包' : offlineScan?.exists === false ? '离线包目录尚未创建' : '还没有离线包' }}
            </span>
            <span class="empty-hint">
              {{ loadingPackages ? '请稍候' : '在镜像列表勾选镜像并导出，生成的 tar 包会出现在这里。' }}
            </span>
            <div v-if="!loadingPackages" class="empty-actions">
              <el-button size="small" type="primary" @click="activeTab = 'images'">前往镜像列表</el-button>
            </div>
          </div>
        </div>
      </el-tab-pane>

      <el-tab-pane v-if="isRemote" label="远程分发" name="remote">
        <RemoteImageDistribution :connection="connection" />
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

    <el-dialog v-model="pullDialogVisible" title="拉取镜像" width="560px" :close-on-click-modal="!pulling">
      <el-form label-width="96px" @submit.prevent>
        <el-form-item label="镜像地址" required>
          <el-input
            v-model="pullForm.reference"
            placeholder="例如 registry.example.com/demo/app:1.0.0"
            clearable
            @keyup.enter="handlePull"
          />
        </el-form-item>
        <el-form-item label="仓库用户名">
          <el-input v-model="pullForm.username" placeholder="公开镜像可留空" autocomplete="off" clearable />
        </el-form-item>
        <el-form-item label="仓库密码">
          <el-input
            v-model="pullForm.password"
            type="password"
            show-password
            placeholder="仅用于本次拉取，不会保存"
            autocomplete="new-password"
          />
        </el-form-item>
      </el-form>
      <div class="dialog-hint">
        镜像拉取到本机 Docker 后，可在镜像列表勾选并导出为离线包。
      </div>
      <template #footer>
        <el-button :disabled="pulling" @click="closePullDialog">取消</el-button>
        <el-button type="primary" :loading="pulling" @click="handlePull">开始拉取</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Download, Refresh } from '@element-plus/icons-vue';
import RemoteImageDistribution from '../components/RemoteImageDistribution.vue';
import {
  exportLocalDockerImages,
  getImageLibrarySettings,
  importOfflinePackages,
  listLocalDockerImages,
  onImageLibraryProgress,
  pullLocalDockerImage,
  removeLocalDockerImages,
  removeOfflinePackages,
  scanOfflinePackages
} from '../api/tauri.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const emit = defineEmits(['go-settings']);

const isRemote = computed(() => props.connection.mode === 'remote');
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
const imageTable = ref(null);
const packageTable = ref(null);
const pullForm = reactive({
  reference: '',
  username: '',
  password: ''
});
let unlistenProgress = null;

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
const offlinePackages = computed(() => offlineScan.value?.files || []);
const totalImageBytes = computed(() =>
  dockerImages.value.reduce((sum, image) => sum + (Number(image.sizeBytes) || 0), 0)
);
const imageSummary = computed(() => {
  if (loadingImages.value && !dockerImages.value.length) {
    return '正在读取';
  }
  return `${dockerImages.value.length} 个镜像 · 共 ${formatSize(totalImageBytes.value)}`;
});
const packageSummary = computed(() => {
  if (!offlineScan.value) {
    return loadingPackages.value ? '正在读取' : '未读取';
  }
  return `${offlinePackages.value.length} 个文件 · 共 ${formatSize(offlineScan.value.totalBytes)}`;
});
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

const formatSize = (bytes) => {
  const value = Number(bytes) || 0;
  if (value >= 1024 * 1024 * 1024) {
    return `${(value / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }
  if (value >= 1024 * 1024) {
    return `${(value / 1024 / 1024).toFixed(1)} MB`;
  }
  if (value >= 1024) {
    return `${(value / 1024).toFixed(0)} KB`;
  }
  return `${value} B`;
};

const formatTime = (ms) => {
  if (!ms) {
    return '-';
  }
  const date = new Date(Number(ms));
  if (Number.isNaN(date.getTime())) {
    return '-';
  }
  const pad = (value) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
};

const shortId = (id) => {
  const value = String(id || '').replace(/^sha256:/, '');
  return value ? value.slice(0, 12) : '-';
};

const imageLabel = (image) =>
  image.exportable ? image.reference : `未标记镜像（${shortId(image.id)}）`;

const loadSettings = async () => {
  loadingSettings.value = true;
  try {
    const settings = await getImageLibrarySettings();
    offlineDir.value = settings.offlineDir || '';
  } catch (error) {
    ElMessage.error(String(error));
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
    imageTable.value?.clearSelection?.();
  } catch (error) {
    dockerImages.value = [];
    imageSelection.value = [];
    imagesError.value = String(error);
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
    packageTable.value?.clearSelection?.();
  } catch (error) {
    offlineScan.value = null;
    packageSelection.value = [];
    packagesError.value = String(error);
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

const openPullDialog = () => {
  pullForm.reference = '';
  pullForm.username = '';
  pullForm.password = '';
  pullDialogVisible.value = true;
};

const closePullDialog = () => {
  if (pulling.value) {
    return;
  }
  pullDialogVisible.value = false;
  pullForm.password = '';
};

const handlePull = async () => {
  if (pulling.value) {
    return;
  }
  const reference = pullForm.reference.trim();
  if (!reference) {
    ElMessage.warning('请填写要拉取的镜像地址');
    return;
  }
  pulling.value = true;
  progress.value = [];
  resultLog.value = '';
  try {
    const output = await pullLocalDockerImage(
      reference,
      pullForm.username.trim() || null,
      pullForm.password || null
    );
    resultLog.value = output;
    resultTitle.value = `已拉取 ${reference}`;
    ElMessage.success('镜像拉取完成');
    pullDialogVisible.value = false;
    pullForm.password = '';
    await loadImages();
  } catch (error) {
    resultLog.value = String(error);
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
    resultLog.value = String(error);
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
    resultLog.value = String(error);
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
    resultLog.value = String(error);
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
    resultLog.value = String(error);
    resultTitle.value = '导入离线包失败';
    ElMessage.error('导入离线包失败');
  } finally {
    importingPackages.value = false;
  }
};

const goSettings = () => {
  emit('go-settings');
};

onMounted(async () => {
  unlistenProgress = await onImageLibraryProgress((payload) => {
    progress.value.push(payload);
  });
  await refreshAll();
});

onUnmounted(() => {
  if (typeof unlistenProgress === 'function') {
    unlistenProgress();
  }
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

.image-card {
  padding: 20px;
  margin-bottom: 18px;
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

.toolbar-row,
.selection-row,
.dir-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.toolbar-row {
  margin-bottom: 12px;
}

.selection-row {
  padding: 10px 12px;
  margin-bottom: 14px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: #fbfcfe;
}

.selection-hint {
  font-size: 12px;
  color: var(--text-secondary);
  margin-right: 4px;
}

.dir-row {
  margin-bottom: 14px;
}

.dir-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.path-text {
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 6px;
  color: var(--primary-color);
  background: rgba(66, 133, 244, 0.08);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  overflow-wrap: anywhere;
}

.inline-hint {
  width: auto;
  margin-top: 0;
}

.field-hint {
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-muted);
}

.error-alert {
  margin-bottom: 14px;
}

.error-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 6px;
  font-size: 12px;
}

.image-table {
  width: 100%;
  background: transparent;
}

.image-table :deep(code) {
  color: var(--primary-color);
  font-size: 11px;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
}

.image-table :deep(.muted-code) {
  color: var(--text-muted);
}

.ref-line {
  display: block;
  line-height: 1.7;
}

.muted-text {
  color: var(--text-muted);
  font-size: 12px;
}

.empty-state {
  min-height: 160px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: center;
  justify-content: center;
  padding: 18px;
  color: var(--text-muted);
  font-size: 12px;
  text-align: center;
  border: 1px dashed var(--border-color);
  border-radius: var(--radius-sm);
}

.empty-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
}

.empty-hint {
  max-width: 520px;
  line-height: 1.6;
}

.empty-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  justify-content: center;
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

.dialog-hint {
  padding: 10px 12px;
  border-radius: var(--radius-sm);
  background: #f7f9fc;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}

.output-log {
  max-height: 260px;
  overflow: auto;
  margin: 0;
  padding: 12px;
  border-radius: var(--radius-sm);
  background: #0b0f17;
  border: 1px solid var(--border-color);
  color: #b9c6d8;
  font-size: 11px;
  line-height: 1.55;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: pre-wrap;
}
</style>
