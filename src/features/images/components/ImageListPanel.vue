<template>
  <div class="card-surface image-card">
    <div class="card-title-bar">
      <h3>本机 Docker 镜像</h3>
      <span class="hint">{{ summary }}</span>
    </div>

    <div class="toolbar-row">
      <el-button :loading="loading" @click="emit('refresh')">
        <el-icon><Refresh /></el-icon>
        <span>刷新镜像</span>
      </el-button>
      <el-button type="primary" @click="emit('pull')">
        <el-icon><Download /></el-icon>
        <span>拉取镜像</span>
      </el-button>
    </div>

    <div class="selection-row">
      <span class="selection-hint">已选择 {{ selectedCount }} 个镜像</span>
      <el-button
        type="primary"
        plain
        :disabled="!selectedCount || busy"
        :loading="exporting"
        @click="emit('export-selected')"
      >
        导出离线包
      </el-button>
      <el-button
        type="danger"
        plain
        :disabled="!selectedCount || busy"
        :loading="deleting"
        @click="emit('delete-selected')"
      >
        删除选中镜像
      </el-button>
      <span class="field-hint inline-hint">离线包保存到 {{ offlineDir || '设置中的目录' }}</span>
    </div>

    <el-alert v-if="error" type="warning" :closable="false" show-icon class="error-alert">
      <template #title>
        <span>{{ error }}</span>
      </template>
      <div class="error-actions">
        <span>请确认 Docker 已安装并启动。</span>
        <el-button size="small" @click="emit('refresh')">重新读取</el-button>
      </div>
    </el-alert>

    <el-table
      v-if="images.length"
      ref="tableRef"
      :data="images"
      row-key="reference"
      size="small"
      class="image-table"
      @selection-change="(rows) => emit('selection-change', rows)"
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
          <code class="muted-code">{{ formatImageId(row.id) }}</code>
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

    <div v-else-if="!error" class="empty-state">
      <span class="icon-tile neutral"><el-icon><Box /></el-icon></span>
      <span class="empty-title">{{ loading ? '正在读取本机镜像' : '本机还没有 Docker 镜像' }}</span>
      <span class="empty-hint">
        {{ loading ? '请稍候' : '填写镜像地址后拉取，或先在目标机上导入离线包。' }}
      </span>
      <div v-if="!loading" class="empty-actions">
        <el-button size="small" type="primary" @click="emit('pull')">拉取镜像</el-button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { Box, Download, Refresh } from '@element-plus/icons-vue';
import { formatImageId, formatSize } from '../../../shared/format.js';

const props = defineProps({
  images: { type: Array, default: () => [] },
  loading: Boolean,
  error: { type: String, default: '' },
  selectedCount: { type: Number, default: 0 },
  busy: Boolean,
  exporting: Boolean,
  deleting: Boolean,
  offlineDir: { type: String, default: '' }
});

const emit = defineEmits([
  'refresh',
  'pull',
  'export-selected',
  'delete-selected',
  'selection-change'
]);

const tableRef = ref(null);

const summary = computed(() => {
  if (props.loading && !props.images.length) {
    return '正在读取';
  }
  const totalBytes = props.images.reduce((sum, image) => sum + (Number(image.sizeBytes) || 0), 0);
  return `${props.images.length} 个镜像 · 共 ${formatSize(totalBytes)}`;
});

defineExpose({
  clearSelection: () => tableRef.value?.clearSelection?.()
});
</script>

<style scoped>
.image-card {
  padding: 20px;
  margin-bottom: 18px;
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
.selection-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.toolbar-row {
  margin-bottom: 12px;
}

.selection-row {
  gap: 12px;
  padding: 12px 14px;
  margin-bottom: 14px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
}

.selection-hint {
  font-size: 12px;
  color: var(--text-secondary);
  margin-right: 4px;
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

.muted-text {
  color: var(--text-muted);
  font-size: 12px;
}

.empty-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  justify-content: center;
}
</style>
