<template>
  <div class="page-view image-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">离线镜像包管理</h2>
        <p class="page-subtitle">扫描本地镜像包、上传到目标机，并按需导入 Docker</p>
      </div>
      <div class="header-actions">
        <el-tag v-if="isRemote" type="primary" effect="plain">
          远程 {{ connection.remoteHost }}
        </el-tag>
        <el-tag v-else type="info" effect="plain">本地部署</el-tag>
        <el-button :loading="scanning" @click="scanLocal">
          <el-icon><FolderOpened /></el-icon>
          <span>扫描本地镜像包</span>
        </el-button>
        <el-button v-if="isRemote" :loading="inspecting" @click="inspectRemote">
          <el-icon><Refresh /></el-icon>
          <span>检测目标机</span>
        </el-button>
      </div>
    </div>

    <el-alert
      v-if="!isRemote"
      title="本地模式直接从当前机器导入镜像"
      description="本地模式无需上传，扫描到镜像包后可直接点击“导入本地镜像”。远程模式会把镜像包上传到目标机后再导入。"
      type="info"
      :closable="false"
      show-icon
      class="mode-alert"
    />

    <div class="card-surface image-card">
      <div class="card-title-bar">
        <h3>本地镜像包</h3>
        <span class="hint">支持选择离线包根目录或其中的 images 目录</span>
      </div>

      <el-form label-width="110px">
        <el-form-item label="本地目录">
          <div class="dir-input-row">
            <el-input v-model="localDir" placeholder="/path/to/deploy-offline-bundle" />
            <el-button @click="pickLocalDir">选择目录</el-button>
          </div>
          <div class="field-hint">
            工具会读取目录下的 images/*.tar 离线镜像包。
          </div>
        </el-form-item>
      </el-form>

      <div v-if="localScan" class="summary-row">
        <el-tag :type="localScan.exists ? 'success' : 'danger'" size="small">
          {{ localScan.exists ? '目录就绪' : '目录不存在' }}
        </el-tag>
        <span class="summary-text">{{ localScan.message }}</span>
        <span v-if="localScan.total_bytes" class="summary-text">
          共 {{ formatSize(localScan.total_bytes) }}
        </span>
        <code class="path-text">{{ localScan.images_dir }}</code>
      </div>

      <el-table
        v-if="localScan && localScan.files.length"
        :data="localScan.files"
        size="small"
        class="image-table"
      >
        <el-table-column prop="name" label="镜像包文件" min-width="170" />
        <el-table-column label="对应镜像" min-width="250">
          <template #default="{ row }">
            <code v-if="row.image_ref">{{ row.image_ref }}</code>
            <span v-else class="muted-text">未识别文件名</span>
          </template>
        </el-table-column>
        <el-table-column label="大小" width="110">
          <template #default="{ row }">{{ formatSize(row.size_bytes) }}</template>
        </el-table-column>
        <el-table-column label="修改时间" width="170">
          <template #default="{ row }">{{ formatTime(row.modified_ms) }}</template>
        </el-table-column>
        <el-table-column v-if="isRemote" label="目标机状态" width="150">
          <template #default="{ row }">
            <span :class="localRowClass(row)">{{ localRowText(row) }}</span>
          </template>
        </el-table-column>
      </el-table>

      <div v-else class="empty-state">
        {{ localScan ? localScan.message : '点击“扫描本地镜像包”读取目录内容' }}
      </div>
    </div>

    <div v-if="isRemote" class="card-surface image-card">
      <div class="card-title-bar">
        <h3>目标机镜像包</h3>
        <span v-if="remote" class="hint">
          {{ remote.exists ? `${remote.files.length} 个文件 · ${formatSize(remote.total_bytes)}` : '镜像目录不存在' }}
        </span>
      </div>

      <div v-if="remote" class="summary-row">
        <el-tag :type="remote.exists ? 'success' : 'danger'" size="small">
          {{ remote.exists ? '目录就绪' : '目录不存在' }}
        </el-tag>
        <el-tag :type="remote.docker_running ? 'success' : 'danger'" size="small">
          {{ remote.docker_running ? 'Docker 运行中' : 'Docker 未运行' }}
        </el-tag>
        <span class="summary-text">剩余空间 {{ formatSize(remote.free_kb * 1024) }}</span>
        <code class="path-text">{{ remote.images_dir }}</code>
      </div>

      <div v-if="remote && remote.issues.length" class="issue-row">
        <span v-for="issue in remote.issues" :key="issue" class="issue-chip">{{ issue }}</span>
      </div>

      <el-table
        v-if="remote && remoteRows.length"
        ref="remoteTable"
        :data="remoteRows"
        size="small"
        class="image-table"
        @selection-change="handleRemoteSelection"
      >
        <el-table-column type="selection" width="44" :selectable="isRowSelectable" />
        <el-table-column prop="name" label="镜像包文件" min-width="170" />
        <el-table-column label="对应镜像" min-width="250">
          <template #default="{ row }">
            <code v-if="row.known">{{ row.image_ref }}</code>
            <span v-else class="muted-text">{{ row.image_ref }}</span>
          </template>
        </el-table-column>
        <el-table-column label="镜像包" width="110">
          <template #default="{ row }">
            <el-tag :type="row.present ? 'success' : 'info'" size="small" effect="plain">
              {{ row.present ? formatSize(row.size_bytes) : '缺失' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="Docker 镜像" width="130">
          <template #default="{ row }">
            <el-tag v-if="row.known" :type="row.loaded ? 'success' : 'warning'" size="small" effect="plain">
              {{ row.loaded ? '已导入' : '未导入' }}
            </el-tag>
            <span v-else class="muted-text">未识别</span>
          </template>
        </el-table-column>
        <el-table-column label="修改时间" width="170">
          <template #default="{ row }">{{ row.present ? formatTime(row.modified_ms) : '-' }}</template>
        </el-table-column>
      </el-table>

      <div v-else class="empty-state">
        {{ remote ? (remote.exists ? '镜像目录为空' : '目标机镜像目录不存在，请先上传本地镜像包') : '点击“检测目标机”读取镜像包与 Docker 镜像状态' }}
      </div>
    </div>

    <div class="card-surface action-card">
      <div class="card-title-bar">
        <h3>操作</h3>
        <span class="hint">上传与导入会逐个文件执行，可实时查看进度</span>
      </div>

      <div class="action-row">
        <template v-if="isRemote">
          <el-button type="primary" :disabled="!canUpload" :loading="uploading" @click="handleUpload(false)">
            上传缺失镜像包
          </el-button>
          <el-button :disabled="!canUpload" :loading="uploading" @click="handleUpload(true)">
            覆盖上传全部
          </el-button>
          <el-button type="success" :disabled="!canImport" :loading="importing" @click="handleImport(false)">
            仅导入未加载镜像
          </el-button>
          <el-button :disabled="!canImport" :loading="importing" @click="handleImport(true)">
            全部重新导入
          </el-button>
          <el-button type="danger" plain :disabled="!canDelete || busy" @click="handleDelete">
            删除选中镜像包
          </el-button>
        </template>
        <template v-else>
          <el-button type="primary" :disabled="!canLocalImport" :loading="importing" @click="handleLocalImport">
            导入本地镜像
          </el-button>
        </template>
      </div>

      <div class="field-hint">
        <template v-if="isRemote">
          上传写入目标机 <code>{{ remoteImagesDir }}</code>；导入执行 <code>docker load</code>。
          删除只移除镜像包文件，不影响已导入的 Docker 镜像。
        </template>
        <template v-else>
          本地导入执行 <code>docker load</code>，不会复制或删除镜像包文件。
        </template>
      </div>
    </div>

    <div v-if="progress.length" class="card-surface progress-card">
      <div class="card-title-bar">
        <h3>执行进度</h3>
        <span class="hint">{{ progressMessage }}</span>
      </div>
      <el-progress :percentage="currentPercent" class="progress-bar" />
      <div class="progress-list">
        <div
          v-for="(item, index) in progress.slice(-8)"
          :key="`${item.file_name}-${index}`"
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
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { FolderOpened, Refresh } from '@element-plus/icons-vue';
import { open } from '@tauri-apps/plugin-dialog';
import {
  importRemoteImageBundle,
  inspectRemoteImageBundle,
  loadOfflineImages,
  onImageManagerProgress,
  removeRemoteImageFiles,
  scanLocalImageBundle,
  uploadRemoteImageBundle
} from '../api/tauri.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const isRemote = computed(() => props.connection.mode === 'remote');
const mediaRoot = computed(() => props.connection.mediaDir || props.connection.bundleDir);
const localDir = ref(
  props.connection.mode === 'remote'
    ? readSavedLocalDir()
    : mediaRoot.value
);
const localScan = ref(null);
const remote = ref(null);
const remoteDir = ref(mediaRoot.value);
const selectedRemoteFiles = ref([]);
const progress = ref([]);
const resultLog = ref('');
const resultTitle = ref('');
const scanning = ref(false);
const inspecting = ref(false);
const uploading = ref(false);
const importing = ref(false);
let unlistenProgress = null;

const busy = computed(() => scanning.value || inspecting.value || uploading.value || importing.value);
const remoteImagesDir = computed(() =>
  `${String(mediaRoot.value || '').replace(/\/+$/, '')}/images`
);

const remoteRows = computed(() => {
  if (!remote.value) {
    return [];
  }
  const rows = [];
  const knownNames = new Set();
  for (const expectation of remote.value.expectations || []) {
    const file = remote.value.files.find((item) => item.name === expectation.file_name);
    rows.push({
      name: expectation.file_name,
      image_ref: expectation.image_ref,
      known: true,
      present: Boolean(file),
      size_bytes: file ? file.size_bytes : 0,
      modified_ms: file ? file.modified_ms : 0,
      loaded: remote.value.docker_images.includes(expectation.image_ref)
    });
    knownNames.add(expectation.file_name);
  }
  for (const file of remote.value.files) {
    if (knownNames.has(file.name)) {
      continue;
    }
    rows.push({
      name: file.name,
      image_ref: file.image_ref || '未识别文件名',
      known: false,
      present: true,
      size_bytes: file.size_bytes,
      modified_ms: file.modified_ms,
      loaded: false
    });
  }
  return rows;
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
  return `${last.current} / ${last.total} · ${last.file_name}`;
});

const canUpload = computed(
  () => isRemote.value && localScan.value?.files?.length > 0 && !busy.value
);
const canImport = computed(
  () =>
    isRemote.value &&
    remoteRows.value.some((row) => row.known && row.present) &&
    !busy.value &&
    remote.value?.docker_running
);
const canDelete = computed(() => selectedRemoteFiles.value.length > 0);
const canLocalImport = computed(() => !isRemote.value && localScan.value?.files?.length > 0 && !busy.value);

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

const remoteFileFor = (name) => remote.value?.files?.find((file) => file.name === name);

const localRowText = (row) => {
  const file = remoteFileFor(row.name);
  if (!file) {
    return '未上传';
  }
  if (file.size_bytes === row.size_bytes) {
    return '已上传';
  }
  return '大小不一致';
};

const localRowClass = (row) => {
  const file = remoteFileFor(row.name);
  if (!file) {
    return 'state-missing';
  }
  return file.size_bytes === row.size_bytes ? 'state-ok' : 'state-warn';
};

const isRowSelectable = (row) => row.present && row.known;

const handleRemoteSelection = (rows) => {
  selectedRemoteFiles.value = rows.map((row) => row.name);
};

const scanLocal = async () => {
  if (!localDir.value.trim()) {
    ElMessage.warning('请先选择本地镜像包目录');
    return;
  }
  scanning.value = true;
  try {
    localScan.value = await scanLocalImageBundle(localDir.value.trim());
    saveLocalDir(localDir.value.trim());
    if (!localScan.value.exists) {
      ElMessage.warning(localScan.value.message);
    }
  } catch (e) {
    localScan.value = null;
    ElMessage.error(`扫描本地镜像包失败: ${e}`);
  } finally {
    scanning.value = false;
  }
};

function readSavedLocalDir() {
  try {
    return localStorage.getItem('image-local-dir') || '';
  } catch {
    return '';
  }
}

function saveLocalDir(dir) {
  try {
    localStorage.setItem('image-local-dir', dir);
  } catch {}
}

const pickLocalDir = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择离线镜像包目录'
    });
    if (typeof selected === 'string' && selected) {
      localDir.value = selected;
      await scanLocal();
    }
  } catch (e) {
    ElMessage.error(`选择目录失败: ${e}`);
  }
};

const inspectRemote = async () => {
  if (!isRemote.value) {
    return;
  }
  inspecting.value = true;
  try {
    remote.value = await inspectRemoteImageBundle(
      props.connection.remoteHost,
      props.connection.remotePort,
      props.connection.remoteUser,
      props.connection.remoteKey || null,
      mediaRoot.value
    );
    remoteDir.value = remote.value.images_dir;
    selectedRemoteFiles.value = [];
  } catch (e) {
    remote.value = null;
    ElMessage.error(`检测目标机镜像包失败: ${e}`);
  } finally {
    inspecting.value = false;
  }
};

const uploadFileList = computed(() => {
  if (!localScan.value?.files?.length) {
    return [];
  }
  return localScan.value.files
    .filter((file) => remoteFileFor(file.name)?.size_bytes !== file.size_bytes)
    .map((file) => file.name);
});

const handleUpload = async (overwrite) => {
  const files = overwrite
    ? localScan.value.files.map((file) => file.name)
    : uploadFileList.value;
  if (!files.length) {
    ElMessage.info('目标机已存在全部镜像包，无需上传');
    return;
  }
  if (overwrite) {
    try {
      await ElMessageBox.confirm(
        `将覆盖目标机 ${remoteImagesDir.value} 中同名镜像包，共 ${files.length} 个文件。是否继续？`,
        '确认覆盖上传',
        { confirmButtonText: '开始上传', cancelButtonText: '取消', type: 'warning' }
      );
    } catch {
      return;
    }
  }

  uploading.value = true;
  resultLog.value = '';
  progress.value = [];
  try {
    const messages = await uploadRemoteImageBundle(
      props.connection.remoteHost,
      props.connection.remotePort,
      props.connection.remoteUser,
      props.connection.remoteKey || null,
      localDir.value.trim(),
      mediaRoot.value,
      files,
      overwrite
    );
    resultLog.value = messages.join('\n');
    resultTitle.value = `已上传 ${files.length} 个文件`;
    ElMessage.success('镜像包上传完成');
    await inspectRemote();
  } catch (e) {
    resultLog.value = String(e);
    resultTitle.value = '上传失败';
    ElMessage.error('镜像包上传失败');
  } finally {
    uploading.value = false;
  }
};

const applyImport = async (files) => {
  importing.value = true;
  resultLog.value = '';
  progress.value = [];
  try {
    const loaded = await importRemoteImageBundle(
      props.connection.remoteHost,
      props.connection.remotePort,
      props.connection.remoteUser,
      props.connection.remoteKey || null,
      mediaRoot.value,
      files
    );
    resultLog.value = loaded.join('\n');
    resultTitle.value = `已导入 ${files.length} 个镜像`;
    ElMessage.success('镜像导入完成');
    await inspectRemote();
  } catch (e) {
    resultLog.value = String(e);
    resultTitle.value = '导入失败';
    ElMessage.error('镜像导入失败');
  } finally {
    importing.value = false;
  }
};

const handleImport = async (reloadAll) => {
  const files = remoteRows.value
    .filter((row) => row.known && row.present && (reloadAll || !row.loaded))
    .map((row) => row.name);
  if (!files.length) {
    ElMessage.info('没有需要导入的镜像');
    return;
  }
  await applyImport(files);
};

const handleLocalImport = async () => {
  importing.value = true;
  resultLog.value = '';
  progress.value = [];
  try {
    const loaded = await loadOfflineImages(localDir.value.trim());
    resultLog.value = loaded.join('\n');
    resultTitle.value = `已导入 ${loaded.length} 个镜像`;
    ElMessage.success('本地镜像导入完成');
  } catch (e) {
    resultLog.value = String(e);
    resultTitle.value = '导入失败';
    ElMessage.error('本地镜像导入失败');
  } finally {
    importing.value = false;
  }
};

const handleDelete = async () => {
  const files = [...selectedRemoteFiles.value];
  if (!files.length) {
    return;
  }
  try {
    await ElMessageBox.confirm(
      `将从目标机删除 ${files.length} 个镜像包文件（${files.join('、')}）。已导入的 Docker 镜像不受影响，但之后需要重新上传才能再次导入。是否继续？`,
      '确认删除镜像包',
      { confirmButtonText: '删除', cancelButtonText: '取消', type: 'warning' }
    );
  } catch {
    return;
  }
  try {
    resultLog.value = await removeRemoteImageFiles(
      props.connection.remoteHost,
      props.connection.remotePort,
      props.connection.remoteUser,
      props.connection.remoteKey || null,
      mediaRoot.value,
      files
    );
    resultTitle.value = '删除完成';
    ElMessage.success('已删除目标机镜像包');
    await inspectRemote();
  } catch (e) {
    resultLog.value = String(e);
    resultTitle.value = '删除失败';
    ElMessage.error('删除目标机镜像包失败');
  }
};

onMounted(async () => {
  unlistenProgress = await onImageManagerProgress((payload) => {
    progress.value.push(payload);
  });
  if (localDir.value.trim()) {
    await scanLocal();
  }
  if (isRemote.value) {
    await inspectRemote();
  }
});

onUnmounted(() => {
  if (typeof unlistenProgress === 'function') {
    unlistenProgress();
  }
});
</script>

<style scoped>
.image-card,
.action-card,
.progress-card,
.result-card {
  padding: 20px;
  margin-bottom: 18px;
}

.mode-alert {
  margin-bottom: 20px;
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

.field-hint {
  width: 100%;
  margin-top: 6px;
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-muted);
}

.dir-input-row {
  display: flex;
  gap: 10px;
  width: 100%;
}

.summary-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.summary-text {
  font-size: 12px;
  color: var(--text-secondary);
}

.path-text {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 6px;
  color: var(--primary-color);
  background: rgba(66, 133, 244, 0.08);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  overflow-wrap: anywhere;
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

.muted-text {
  color: var(--text-muted);
  font-size: 12px;
}

.state-ok {
  color: var(--success-color);
  font-size: 12px;
}

.state-warn {
  color: var(--warning-color);
  font-size: 12px;
}

.state-missing {
  color: var(--text-muted);
  font-size: 12px;
}

.issue-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 12px;
}

.issue-chip {
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 999px;
  color: var(--warning-color);
  background: rgba(242, 196, 109, 0.1);
  border: 1px solid rgba(242, 196, 109, 0.22);
}

.action-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.empty-state {
  min-height: 120px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  font-size: 12px;
  border: 1px dashed var(--border-color);
  border-radius: var(--radius-sm);
}

.progress-bar {
  margin-bottom: 12px;
}

.progress-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
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
  max-height: 240px;
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
