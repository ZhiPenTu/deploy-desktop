<template>
  <div class="remote-distribution">
    <el-alert
      title="把离线包上传到目标机后再导入 Docker"
      description="先在本机镜像列表导出离线包，再回到这里上传。导入过程不会修改或删除目标机上的离线包文件。"
      type="info"
      :closable="false"
      show-icon
      class="remote-alert"
    />

    <div class="card-surface remote-card">
      <div class="card-title-bar">
        <h3>离线包来源</h3>
        <span class="hint">{{ offlineCountText }}</span>
      </div>
      <div class="summary-row">
        <span class="summary-text">保存目录</span>
        <code class="path-text">{{ offlineDir || '正在读取' }}</code>
        <el-button size="small" :loading="loadingLocal" @click="loadLocal">
          <el-icon><Refresh /></el-icon>
          <span>刷新离线包</span>
        </el-button>
      </div>
      <div class="field-hint">
        这里列出设置中保存目录下的离线包。没有离线包时，请先在镜像列表勾选镜像并导出。
      </div>
    </div>

    <div class="card-surface remote-card">
      <div class="card-title-bar">
        <h3>目标机镜像包</h3>
        <span v-if="remote" class="hint">
          {{ remote.exists ? `${remote.files.length} 个文件 · ${formatSize(remote.total_bytes)}` : '镜像目录未就绪' }}
        </span>
      </div>

      <el-form label-width="110px">
        <el-form-item label="目标机目录">
          <div class="dir-input-row">
            <el-input v-model="remoteDirPath" placeholder="/home/deploy/bundle" />
            <el-button :loading="inspecting" @click="inspectRemote()">
              <el-icon><Refresh /></el-icon>
              <span>检测目标机</span>
            </el-button>
          </div>
          <div class="field-hint">
            镜像包会上传到该目录下的 images/；留空时使用目标部署目录下的 bundle/。
          </div>
        </el-form-item>
      </el-form>

      <el-alert
        v-if="remoteError"
        type="warning"
        :closable="false"
        show-icon
        :title="remoteError"
        class="remote-alert"
      />

      <div v-if="remote" class="summary-row">
        <el-tag :type="remote.docker_running ? 'success' : 'warning'" size="small" effect="plain">
          {{ remote.docker_running ? 'Docker 已运行' : 'Docker 未运行' }}
        </el-tag>
        <span class="summary-text">{{ remote.files.length }} 个镜像包</span>
        <span class="summary-text">可用空间 {{ formatSize(remote.free_kb * 1024) }}</span>
        <code class="path-text">{{ remote.images_dir }}</code>
      </div>

      <div v-if="remote && remote.issues.length" class="issue-row">
        <span v-for="issue in remote.issues" :key="issue" class="issue-chip">{{ issue }}</span>
      </div>

      <el-table
        v-if="remote && remoteRows.length"
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
        {{ remote ? (remote.exists ? '目标机镜像目录为空' : '目标机镜像目录未就绪，请先上传离线包') : '点击“检测目标机”读取镜像包与 Docker 镜像状态' }}
      </div>
    </div>

    <div class="card-surface remote-card">
      <div class="card-title-bar">
        <h3>操作</h3>
        <span class="hint">上传与导入会逐个文件执行，可实时查看进度</span>
      </div>

      <div class="action-row">
        <el-button type="primary" :disabled="!canUpload" :loading="uploading" @click="handleUpload(false)">
          上传缺失离线包
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
      </div>

      <div class="field-hint">
        上传写入目标机 <code>{{ remoteImagesDir }}</code>；导入执行 docker load。
        删除只移除目标机镜像包文件，不影响已导入的 Docker 镜像。
      </div>
    </div>

    <div v-if="progress.length" class="card-surface remote-card">
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

    <div v-if="resultLog" class="card-surface remote-card">
      <div class="card-title-bar">
        <h3>执行结果</h3>
        <span class="hint">{{ resultTitle }}</span>
      </div>
      <pre class="output-log">{{ resultLog }}</pre>
    </div>
  </div>
</template>

<script setup>
import { useDesktopProgress } from '../../../shared/composables/useDesktopProgress.js';
import { computed, onMounted, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Refresh } from '@element-plus/icons-vue';
import {
  importRemoteImageBundle,
  inspectRemoteImageBundle,
  onImageManagerProgress,
  removeRemoteImageFiles,
  scanOfflinePackages,
  uploadRemoteImageBundle
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const offlineScan = ref(null);
const loadingLocal = ref(false);
const remote = ref(null);
const remoteError = ref('');
const remoteDirPath = ref(
  String(props.connection.remoteBundleDir || '').trim() ||
    `${String(props.connection.deployDir || '/opt/deploy').replace(/\/+$/, '')}/bundle`
);
const selectedRemoteFiles = ref([]);
const progress = ref([]);
const resultLog = ref('');
const resultTitle = ref('');
const inspecting = ref(false);
const uploading = ref(false);
const importing = ref(false);

const offlineDir = computed(() => offlineScan.value?.offlineDir || '');
const offlineFiles = computed(() => offlineScan.value?.files || []);
const localImageMap = computed(() => {
  const map = new Map();
  for (const file of offlineFiles.value) {
    if (file.imageRefs?.length) {
      map.set(file.name, file.imageRefs.join('、'));
    }
  }
  return map;
});
const offlineCountText = computed(() =>
  offlineScan.value
    ? `${offlineFiles.value.length} 个离线包 · ${formatSize(offlineScan.value.totalBytes)}`
    : '正在读取离线包目录'
);
const busy = computed(() => loadingLocal.value || inspecting.value || uploading.value || importing.value);
const remoteImagesDir = computed(() =>
  `${String(remoteDirPath.value || '').replace(/\/+$/, '')}/images`
);
const canUpload = computed(
  () => offlineFiles.value.length > 0 && !busy.value
);
const canImport = computed(
  () =>
    Boolean(remote.value?.docker_running) &&
    remoteRows.value.some((row) => row.present) &&
    !busy.value
);
const canDelete = computed(() => selectedRemoteFiles.value.length > 0);

const remoteRows = computed(() => {
  if (!remote.value) {
    return [];
  }
  const rows = [];
  const knownNames = new Set();
  for (const expectation of remote.value.expectations || []) {
    const file = remote.value.files.find((item) => item.name === expectation.file_name);
    const imageRef = localImageMap.value.get(expectation.file_name) || expectation.image_ref;
    rows.push({
      name: expectation.file_name,
      image_ref: imageRef,
      known: Boolean(imageRef),
      present: Boolean(file),
      size_bytes: file ? file.size_bytes : 0,
      modified_ms: file ? file.modified_ms : 0,
      loaded: isImageLoaded(imageRef)
    });
    knownNames.add(expectation.file_name);
  }
  for (const file of remote.value.files) {
    if (knownNames.has(file.name)) {
      continue;
    }
    const imageRef = localImageMap.value.get(file.name) || file.image_ref;
    rows.push({
      name: file.name,
      image_ref: imageRef || '未识别文件名',
      known: Boolean(imageRef),
      present: true,
      size_bytes: file.size_bytes,
      modified_ms: file.modified_ms,
      loaded: isImageLoaded(imageRef)
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

const uploadFileList = computed(() => {
  if (!offlineFiles.value.length) {
    return [];
  }
  return offlineFiles.value
    .filter((file) => remoteFileFor(file.name)?.size_bytes !== file.size_bytes)
    .map((file) => file.name);
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

const remoteFileFor = (name) => remote.value?.files?.find((file) => file.name === name);

const isImageLoaded = (imageRef) => {
  const refs = String(imageRef || '')
    .split('、')
    .map((item) => item.trim())
    .filter(Boolean);
  if (!refs.length || !remote.value?.docker_images) {
    return false;
  }
  return refs.every((ref) => remote.value.docker_images.includes(ref));
};

const isRowSelectable = (row) => row.present && row.known;

const handleRemoteSelection = (rows) => {
  selectedRemoteFiles.value = rows.map((row) => row.name);
};

const loadLocal = async () => {
  loadingLocal.value = true;
  try {
    offlineScan.value = await scanOfflinePackages();
  } catch (error) {
    offlineScan.value = null;
    ElMessage.error(errorText(error));
  } finally {
    loadingLocal.value = false;
  }
};

const inspectRemote = async ({ silent = false } = {}) => {
  const host = String(props.connection.remoteHost || '').trim();
  if (!host) {
    remote.value = null;
    remoteError.value = '';
    if (!silent) {
      ElMessage.warning('请先在部署向导中填写目标主机');
    }
    return;
  }
  inspecting.value = true;
  remoteError.value = '';
  try {
    remote.value = await inspectRemoteImageBundle(
      host,
      props.connection.remotePort,
      props.connection.remoteUser,
      props.connection.remoteKey || null,
      remoteDirPath.value
    );
    selectedRemoteFiles.value = [];
  } catch (error) {
    remote.value = null;
    remoteError.value = errorText(error);
    if (!silent) {
      ElMessage.error('检测目标机镜像包失败');
    }
  } finally {
    inspecting.value = false;
  }
};

const handleUpload = async (overwrite) => {
  const files = overwrite
    ? offlineFiles.value.map((file) => file.name)
    : uploadFileList.value;
  if (!files.length) {
    ElMessage.info('目标机已存在全部离线包，无需上传');
    return;
  }
  if (!offlineDir.value) {
    ElMessage.warning('未读取到离线包目录，请先刷新离线包');
    return;
  }
  if (overwrite) {
    try {
      await ElMessageBox.confirm(
        `将覆盖目标机 ${remoteImagesDir.value} 中同名离线包，共 ${files.length} 个文件。是否继续？`,
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
      offlineDir.value,
      remoteDirPath.value,
      files,
      overwrite
    );
    resultLog.value = messages.join('\n');
    resultTitle.value = `已上传 ${files.length} 个文件`;
    ElMessage.success('离线包上传完成');
    await inspectRemote();
  } catch (error) {
    resultLog.value = errorText(error);
    resultTitle.value = '上传失败';
    ElMessage.error('离线包上传失败');
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
      remoteDirPath.value,
      files
    );
    resultLog.value = loaded.join('\n');
    resultTitle.value = `已导入 ${files.length} 个镜像包`;
    ElMessage.success('镜像导入完成');
    await inspectRemote();
  } catch (error) {
    resultLog.value = errorText(error);
    resultTitle.value = '导入失败';
    ElMessage.error('镜像导入失败');
  } finally {
    importing.value = false;
  }
};

const handleImport = async (reloadAll) => {
  const files = remoteRows.value
    .filter((row) => row.present && (reloadAll || !row.loaded))
    .map((row) => row.name);
  if (!files.length) {
    ElMessage.info('没有需要导入的镜像');
    return;
  }
  await applyImport(files);
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
      remoteDirPath.value,
      files
    );
    resultTitle.value = '删除完成';
    ElMessage.success('已删除目标机镜像包');
    await inspectRemote();
  } catch (error) {
    resultLog.value = errorText(error);
    resultTitle.value = '删除失败';
    ElMessage.error('删除目标机镜像包失败');
  }
};

useDesktopProgress(onImageManagerProgress, (payload) => {
  progress.value.push(payload);
}, () => ElMessage.warning('进度暂不可用，请等待执行结果。'));

onMounted(async () => {
  await loadLocal();
  await inspectRemote({ silent: true });
});
</script>

<style scoped>
.remote-card {
  padding: 20px;
  margin-bottom: 18px;
}

.remote-alert {
  margin-bottom: 12px;
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
