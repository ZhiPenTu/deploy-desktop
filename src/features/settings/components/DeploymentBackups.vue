<template>
  <div class="card-surface settings-card">
    <div class="card-title-bar">
      <h3>部署备份</h3>
      <span class="hint">更新和部署前保存的回滚副本，勾选后可删除</span>
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
        <span class="info-label">当前部署版本</span>
        <span class="info-value">{{ currentVersionText }}</span>
        <span class="info-label">占用空间</span>
        <span class="info-value">{{ formatSize(overview.totalBytes) }}</span>
        <span class="info-label">备份数量</span>
        <span class="info-value">{{ backups.length }} 项</span>
        <span class="info-label">最近备份</span>
        <span class="info-value">{{ latestText }}</span>
      </div>

      <el-table
        v-if="backups.length"
        ref="tableRef"
        :data="backups"
        size="small"
        row-key="key"
        class="backup-table"
        @selection-change="onSelectionChange"
      >
        <el-table-column type="selection" width="44" />
        <el-table-column label="版本" min-width="110">
          <template #default="{ row }">
            <span :class="{ 'muted-text': !row.version }">{{ versionText(row) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="类型" width="130">
          <template #default="{ row }">{{ kindText(row.kind) }}</template>
        </el-table-column>
        <el-table-column label="备份时间" width="170">
          <template #default="{ row }">{{ formatTimestamp(row.modifiedAt) }}</template>
        </el-table-column>
        <el-table-column label="大小" width="110" align="right">
          <template #default="{ row }">{{ formatSize(row.bytes) }}</template>
        </el-table-column>
      </el-table>

      <div v-else-if="!loading" class="backup-empty">
        <span>当前没有可清理的部署备份。</span>
      </div>

      <p class="field-hint">
        只删除勾选的备份，不会停止容器或删除数据卷；未完成、待恢复和恢复中的部署不会出现在列表中。较早的备份没有版本记录，会显示“未记录”。
      </p>

      <div class="card-actions">
        <el-button
          type="danger"
          plain
          :loading="cleaning"
          :disabled="loading || cleaning || !selected.length"
          @click="cleanup"
        >
          <el-icon><Delete /></el-icon>
          <span>删除选中{{ selected.length ? `（${selected.length}）` : '' }}</span>
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
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Delete, FolderOpened, Refresh } from '@element-plus/icons-vue';
import {
  inspectDeploymentBackups,
  cleanDeploymentBackups,
  inspectRemoteDeploymentBackups,
  cleanRemoteDeploymentBackups
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';
import { formatSize, formatTimestamp } from '../../../shared/format.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const emptyOverview = () => ({
  currentVersion: '',
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
const selected = ref([]);
const tableRef = ref(null);
let loadToken = 0;

const installDir = computed(() => String(props.connection?.installDir || '').trim());
const isRemote = computed(() => props.connection?.mode === 'remote');
const backups = computed(() =>
  [...overview.value.updateBackups, ...overview.value.transactionSnapshots]
    .map((entry) => ({ ...entry, key: `${entry.kind}:${entry.name}` }))
    .sort((left, right) => (right.modifiedAt || 0) - (left.modifiedAt || 0))
);
const latestAt = computed(() => backups.value[0]?.modifiedAt || 0);
const latestText = computed(() => (latestAt.value ? formatTimestamp(latestAt.value) : '无'));
const currentVersionText = computed(() => {
  const version = String(overview.value.currentVersion || '').trim();
  if (!version) return '未记录';
  return /^\d/.test(version) ? `v${version}` : version;
});

function versionText(row) {
  if (!row.version) return '未记录';
  return /^\d/.test(row.version) ? `v${row.version}` : row.version;
}

function kindText(kind) {
  return kind === 'update' ? '更新前备份' : '部署事务快照';
}

function onSelectionChange(rows) {
  selected.value = Array.isArray(rows) ? rows : [];
}

function clearSelection() {
  selected.value = [];
  tableRef.value?.clearSelection();
}

async function load() {
  const token = ++loadToken;
  overview.value = emptyOverview();
  readError.value = '';
  clearSelection();
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
    await nextTick();
    clearSelection();
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
  if (loading.value || !selected.value.length) {
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
  const selections = selected.value.map((row) => ({ kind: row.kind, name: row.name }));
  const bytes = selected.value.reduce((total, row) => total + (row.bytes || 0), 0);
  const target = request.remote
    ? `${request.host || '目标主机'} 的 ${request.installDir}`
    : request.installDir;
  try {
    await ElMessageBox.confirm(
      `将删除选中的 ${selections.length} 项备份，预计释放 ${formatSize(bytes)}。\n\n目标：${target}\n删除后无法再用这些副本回滚，其他备份不受影响。`,
      '删除部署备份',
      {
        type: 'warning',
        confirmButtonText: '删除',
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
          request.installDir,
          selections
        )
      : await cleanDeploymentBackups(request.installDir, selections);
    const skipped = result.skipped ? `，${result.skipped} 项已跳过` : '';
    ElMessage.success(
      `已删除 ${result.removed} 项部署备份，释放 ${formatSize(result.freedBytes)}${skipped}`
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

.backup-table {
  margin-bottom: 12px;
}

.muted-text {
  color: var(--text-muted);
}

.backup-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 72px;
  margin-bottom: 12px;
  border: 1px dashed var(--border-color);
  border-radius: 8px;
  font-size: 13px;
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
