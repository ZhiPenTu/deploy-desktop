<template>
  <div class="usb-read">
    <div class="card-surface usb-card">
      <div class="card-title-bar">
        <div>
          <h3>介质内容</h3>
          <span class="hint">共 {{ entries.length }} 项，读取前会按介质清单校验文件</span>
        </div>
        <el-button :loading="loading" :disabled="busy" @click="emit('refresh')">
          <el-icon><Refresh /></el-icon>
          <span>刷新介质</span>
        </el-button>
      </div>

      <div v-if="progress" class="read-progress">
        <el-progress :percentage="progress.percent" :stroke-width="8" :show-text="false" />
        <span class="progress-text">
          {{ progress.message }}<template v-if="progress.label">：{{ progress.label }}</template>
        </span>
      </div>

      <el-alert
        v-if="verifiedMessage"
        :title="verifiedMessage"
        type="info"
        :closable="false"
        show-icon
      />

      <el-table v-if="entries.length" :data="entries" row-key="id" size="small" class="usb-table">
        <el-table-column label="类型" width="90">
          <template #default="{ row }">
            <el-tag size="small" effect="plain" :type="row.kind === 'artifact' ? 'primary' : 'info'">
              {{ kindLabel(row.kind) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="名称" min-width="240">
          <template #default="{ row }">
            <div class="name-cell">
              <span>{{ row.displayName || row.label }}</span>
              <code v-for="image in row.images" :key="image" class="ref-line">{{ image }}</code>
            </div>
          </template>
        </el-table-column>
        <el-table-column label="版本" width="150">
          <template #default="{ row }">
            <span v-if="row.version">{{ row.version }}</span>
            <span v-else class="muted-text">－</span>
          </template>
        </el-table-column>
        <el-table-column label="大小" width="110">
          <template #default="{ row }">{{ formatSize(row.sizeBytes) }}</template>
        </el-table-column>
        <el-table-column label="写入时间" width="170">
          <template #default="{ row }">{{ formatTime(row.addedAt) }}</template>
        </el-table-column>
        <el-table-column label="状态" min-width="180">
          <template #default="{ row }">
            <div class="status-cell">
              <el-tag v-if="row.present" size="small" type="success" effect="plain">可读取</el-tag>
              <el-tag v-else size="small" type="danger" effect="plain">不可读取</el-tag>
              <span v-if="row.issue" class="issue-text">{{ row.issue }}</span>
            </div>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="150" align="right">
          <template #default="{ row }">
            <el-button
              v-if="row.kind === 'artifact'"
              size="small"
              type="primary"
              plain
              :disabled="!desktopAvailable || !row.present || busy"
              :loading="deployingId === row.id"
              @click="deploy(row)"
            >
              用于部署
            </el-button>
            <el-button
              v-else
              size="small"
              type="primary"
              plain
              :disabled="!desktopAvailable || !row.present || busy"
              :loading="importingId === row.id"
              @click="importImage(row)"
            >
              复制到离线目录
            </el-button>
          </template>
        </el-table-column>
      </el-table>

      <div v-else-if="!loading && !error" class="empty-state">
        <span class="empty-title">介质中还没有可读取的内容</span>
        <span class="empty-hint">先在「写入 USB」中选择本机交付包或镜像包写入，再在目标电脑上从这里读取。</span>
      </div>
    </div>

    <div class="read-tip">
      <span>交付包会解压到本机并进入部署向导；镜像包会复制到设置的离线包目录，可在「镜像与离线包」中导入 Docker。</span>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { Refresh } from '@element-plus/icons-vue';
import {
  importUsbImage,
  prepareLocalArtifactBundle,
  verifyUsbEntry
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';
import { formatSize, formatTimestamp } from '../../../shared/format.js';

const props = defineProps({
  entries: { type: Array, default: () => [] },
  loading: { type: Boolean, default: false },
  error: { type: String, default: '' },
  mediaDir: { type: String, default: '' },
  desktopAvailable: { type: Boolean, default: false }
});

const emit = defineEmits(['refresh', 'deploy', 'imported', 'busy-change']);

const deployingId = ref('');
const importingId = ref('');
const progress = ref(null);
const verifiedMessage = ref('');

const busy = computed(() => Boolean(deployingId.value || importingId.value));

watch(busy, (value) => emit('busy-change', value), { flush: 'sync' });

function kindLabel(kind) {
  return kind === 'artifact' ? '交付包' : '镜像包';
}

function formatTime(value) {
  if (!value) return '－';
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? '－' : formatTimestamp(parsed);
}

async function deploy(row) {
  if (busy.value) return;
  deployingId.value = row.id;
  progress.value = { percent: 0, message: '正在校验交付包', label: row.label };
  verifiedMessage.value = '';
  try {
    const check = await verifyUsbEntry(props.mediaDir, row.id);
    if (!check.valid) {
      verifiedMessage.value = check.message;
      ElMessage.error(check.message);
      return;
    }
    progress.value = { percent: 40, message: '正在解压交付包', label: row.label };
    const bundle = await prepareLocalArtifactBundle(row.filePath);
    progress.value = { percent: 100, message: '已准备交付包', label: row.label };
    emit('deploy', {
      dir: bundle.dir,
      version: bundle.version || row.version,
      label: row.displayName || row.label
    });
  } catch (deployError) {
    ElMessage.error(errorText(deployError));
  } finally {
    deployingId.value = '';
    progress.value = null;
  }
}

async function importImage(row) {
  if (busy.value) return;
  importingId.value = row.id;
  progress.value = { percent: 0, message: '正在校验镜像包', label: row.label };
  verifiedMessage.value = '';
  try {
    const result = await importUsbImage(props.mediaDir, row.id, (payload) => {
      progress.value = {
        percent: Number(payload?.percent) || 0,
        message: payload?.message || '正在复制镜像包',
        label: row.label
      };
    });
    verifiedMessage.value = `${result.message}：${result.path}`;
    emit('imported');
    ElMessage.success(result.message);
  } catch (importError) {
    ElMessage.error(errorText(importError));
  } finally {
    importingId.value = '';
    progress.value = null;
  }
}
</script>

<style scoped>
.usb-read {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.usb-card {
  padding: 18px 20px;
}

.usb-table {
  margin-top: 4px;
}

.name-cell {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.ref-line {
  font-size: 11.5px;
}

.status-cell {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.issue-text {
  font-size: 11.5px;
  color: var(--text-muted);
}

.read-progress {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 12px;
}

.progress-text {
  font-size: 12px;
  color: var(--text-muted);
}

.read-tip {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-muted);
}

.empty-state {
  margin-top: 8px;
}

@media (max-width: 760px) {
  .usb-card {
    padding: 16px;
  }
}
</style>
