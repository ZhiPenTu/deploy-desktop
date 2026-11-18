<template>
  <div class="card-surface settings-card">
    <div class="card-title-bar">
      <h3>部署备份</h3>
      <span class="hint">更新和部署前保存的回滚副本，占用目标部署目录的空间</span>
    </div>

    <div v-if="!installDir" class="empty-state">
      <span class="icon-tile neutral"><el-icon><FolderOpened /></el-icon></span>
      <span class="empty-title">尚未设置部署目录</span>
      <span class="empty-hint">在项目管理中填写部署目录后，这里会显示可以清理的部署备份。</span>
    </div>

    <template v-else>
      <el-alert
        v-if="readError"
        :title="readError"
        type="error"
        :closable="false"
        show-icon
        class="backup-error"
      />

      <div class="info-grid">
        <span class="info-label">占用空间</span>
        <span class="info-value">{{ formatSize(overview.totalBytes) }}</span>
        <span class="info-label">备份数量</span>
        <span class="info-value">{{ totalCount }} 项</span>
        <span class="info-label">最近备份</span>
        <span class="info-value">{{ latestText }}</span>
      </div>

      <div class="backup-breakdown">
        <div class="backup-line">
          <span>更新前文件备份</span>
          <span>{{ overview.updateBackups.length }} 项 · {{ formatSize(overview.updateBytes) }}</span>
        </div>
        <div class="backup-line">
          <span>已完成部署的事务快照</span>
          <span>{{ overview.transactionSnapshots.length }} 项 · {{ formatSize(overview.transactionBytes) }}</span>
        </div>
      </div>

      <p class="field-hint">
        清理只删除当前部署目录内的回滚副本，不会停止容器或删除数据卷；最近一次更新备份会保留，未完成、待恢复和恢复中的部署不会被清理。
      </p>

      <div class="card-actions">
        <el-button
          type="danger"
          plain
          :loading="cleaning"
          :disabled="loading || cleaning || totalCount === 0"
          @click="cleanup"
        >
          <el-icon><Delete /></el-icon>
          <span>清理部署备份</span>
        </el-button>
        <el-button :loading="loading" @click="load">
          <el-icon><Refresh /></el-icon>
          <span>刷新</span>
        </el-button>
      </div>
    </template>
  </div>
</template>

<script setup>
import { computed, onMounted, ref, watch } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Delete, FolderOpened, Refresh } from '@element-plus/icons-vue';
import {
  inspectDeploymentBackups,
  cleanDeploymentBackups,
  inspectRemoteDeploymentBackups,
  cleanRemoteDeploymentBackups
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';
import { formatSize, formatShortTimestamp } from '../../../shared/format.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const emptyOverview = () => ({
  updateBackups: [],
  transactionSnapshots: [],
  updateBytes: 0,
  transactionBytes: 0,
  totalBytes: 0
});

const overview = ref(emptyOverview());
const loading = ref(false);
const cleaning = ref(false);
const readError = ref('');
let loadToken = 0;

const installDir = computed(() => String(props.connection?.installDir || '').trim());
const isRemote = computed(() => props.connection?.mode === 'remote');
const totalCount = computed(
  () => overview.value.updateBackups.length + overview.value.transactionSnapshots.length
);
const latestAt = computed(() => {
  const entries = [
    ...overview.value.updateBackups,
    ...overview.value.transactionSnapshots
  ];
  return entries.reduce((latest, entry) => Math.max(latest, entry.modifiedAt || 0), 0);
});
const latestText = computed(() => (latestAt.value ? formatShortTimestamp(latestAt.value) : '无'));
const keepCount = computed(() => (overview.value.updateBackups.length ? 1 : 0));
const expectedCount = computed(() => Math.max(0, totalCount.value - keepCount.value));
const expectedBytes = computed(() => {
  const kept = keepCount.value ? overview.value.updateBackups[0]?.bytes || 0 : 0;
  return Math.max(0, overview.value.totalBytes - kept);
});

async function load() {
  const token = ++loadToken;
  overview.value = emptyOverview();
  readError.value = '';
  if (!installDir.value) {
    loading.value = false;
    return;
  }
  if (isRemote.value && !props.connection.remoteHost) {
    readError.value = '当前项目还没有配置目标主机地址，请先在项目管理中补全。';
    loading.value = false;
    return;
  }
  loading.value = true;
  try {
    const result = isRemote.value
      ? await inspectRemoteDeploymentBackups(
          props.connection.remoteHost,
          props.connection.remotePort || 22,
          props.connection.remoteUser || 'root',
          props.connection.remoteKey || null,
          installDir.value
        )
      : await inspectDeploymentBackups(installDir.value);
    if (token !== loadToken) return;
    overview.value = result;
  } catch (error) {
    if (token !== loadToken) return;
    readError.value = errorText(error);
  } finally {
    if (token === loadToken) {
      loading.value = false;
    }
  }
}

async function cleanup() {
  if (loading.value || !totalCount.value) {
    return;
  }
  const request = {
    installDir: installDir.value,
    remote: isRemote.value,
    host: props.connection.remoteHost || '',
    port: props.connection.remotePort || 22,
    user: props.connection.remoteUser || 'root',
    keyPath: props.connection.remoteKey || null
  };
  const target = request.remote
    ? `${request.host || '目标主机'} 的 ${request.installDir}`
    : request.installDir;
  const keepHint = keepCount.value ? '最近一次更新备份会保留。' : '';
  try {
    await ElMessageBox.confirm(
      `将清理 ${expectedCount.value} 项备份，预计释放 ${formatSize(expectedBytes.value)}。\n\n目标：${target}\n${keepHint}清理后无法再用这些副本回滚，未完成的部署不会受影响。`,
      '清理部署备份',
      {
        type: 'warning',
        confirmButtonText: '清理',
        cancelButtonText: '取消'
      }
    );
  } catch {
    return;
  }

  cleaning.value = true;
  try {
    const result = request.remote
      ? await cleanRemoteDeploymentBackups(
          request.host,
          request.port,
          request.user,
          request.keyPath,
          request.installDir
        )
      : await cleanDeploymentBackups(request.installDir);
    ElMessage.success(
      `已清理 ${result.removed} 项部署备份，释放 ${formatSize(result.freedBytes)}`
    );
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    cleaning.value = false;
    await load();
  }
}

watch(
  () => [
    props.connection.mode,
    props.connection.installDir,
    props.connection.remoteHost,
    props.connection.remotePort,
    props.connection.remoteUser,
    props.connection.remoteKey
  ],
  load
);

onMounted(load);
</script>

<style scoped>
.backup-error {
  margin-bottom: 14px;
}

.info-grid {
  display: grid;
  grid-template-columns: 96px minmax(0, 1fr);
  gap: 10px 16px;
  align-items: center;
  margin-bottom: 16px;
  font-size: 13px;
}

.info-label {
  color: var(--text-muted);
}

.info-value {
  color: var(--text-main);
  word-break: break-all;
}

.backup-breakdown {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px;
  margin-bottom: 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--surface-subtle, transparent);
  font-size: 13px;
}

.backup-line {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  color: var(--text-muted);
}

.field-hint {
  margin: 0 0 12px;
  font-size: 12px;
  line-height: 1.7;
  color: var(--text-muted);
}

.card-actions {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  margin-top: 16px;
}

.card-actions :deep(.el-button + .el-button) {
  margin-left: 0;
}
</style>
