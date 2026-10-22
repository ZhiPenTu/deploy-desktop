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

    <RemoteOfflineSourceCard
      :dir="offlineDir"
      :count-text="offlineCountText"
      :loading="loadingLocal"
      @refresh="loadLocal"
    />

    <RemoteBundlePanel
      v-model:dir-path="remoteDirPath"
      :remote="remote"
      :error="remoteError"
      :rows="remoteRows"
      :inspecting="inspecting"
      @inspect="inspectRemote()"
      @selection-change="handleRemoteSelection"
    />

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

    <RemoteProgressCard
      v-if="progress.length"
      :progress="progress"
      :percent="currentPercent"
      :message="progressMessage"
    />

    <RemoteResultCard v-if="resultLog" :log="resultLog" :title="resultTitle" />
  </div>
</template>

<script setup>
import { useDesktopProgress } from '../../../shared/composables/useDesktopProgress.js';
import { computed, onMounted, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import {
  importRemoteImageBundle,
  inspectRemoteImageBundle,
  onImageManagerProgress,
  removeRemoteImageFiles,
  scanOfflinePackages,
  uploadRemoteImageBundle
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';
import { formatSize } from '../../../shared/format.js';
import RemoteBundlePanel from './RemoteBundlePanel.vue';
import RemoteOfflineSourceCard from './RemoteOfflineSourceCard.vue';
import RemoteProgressCard from './RemoteProgressCard.vue';
import RemoteResultCard from './RemoteResultCard.vue';

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

.action-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
</style>
