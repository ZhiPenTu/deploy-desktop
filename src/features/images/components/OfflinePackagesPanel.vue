<template>
  <div class="card-surface image-card">
    <div class="card-title-bar">
      <h3>离线镜像包</h3>
      <span class="hint">{{ summary }}</span>
    </div>

    <div class="dir-row">
      <span class="dir-label">保存目录</span>
      <code class="path-text">{{ directory }}</code>
      <el-button size="small" @click="emit('edit-directory')">修改目录</el-button>
    </div>

    <div class="selection-row">
      <span class="selection-hint">已选择 {{ selectedCount }} 个离线包</span>
      <el-button
        type="primary"
        plain
        :disabled="!selectedCount || busy"
        :loading="importing"
        @click="emit('import-selected')"
      >
        导入到本机 Docker
      </el-button>
      <el-button
        type="danger"
        plain
        :disabled="!selectedCount || busy"
        :loading="deleting"
        @click="emit('delete-selected')"
      >
        删除选中离线包
      </el-button>
      <el-button :loading="loading" @click="emit('refresh')">
        <el-icon><Refresh /></el-icon>
        <span>刷新离线包</span>
      </el-button>
      <el-button @click="emit('go-images')">从镜像列表导出</el-button>
    </div>

    <el-alert v-if="error" type="warning" :closable="false" show-icon :title="error" class="error-alert" />

    <el-table
      v-if="files.length"
      ref="tableRef"
      :data="files"
      row-key="name"
      size="small"
      class="image-table"
      @selection-change="(rows) => emit('selection-change', rows)"
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
        <template #default="{ row }">{{ formatTimestamp(row.modifiedMs) }}</template>
      </el-table-column>
    </el-table>

    <div v-else-if="!error" class="empty-state">
      <span class="icon-tile neutral"><el-icon><Files /></el-icon></span>
      <span class="empty-title">{{ emptyTitle }}</span>
      <span class="empty-hint">
        {{ loading ? '请稍候' : '在镜像列表勾选镜像并导出，生成的 tar 包会出现在这里。' }}
      </span>
      <div v-if="!loading" class="empty-actions">
        <el-button size="small" type="primary" @click="emit('go-images')">前往镜像列表</el-button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { Files, Refresh } from '@element-plus/icons-vue';
import { formatSize, formatTimestamp } from '../../../shared/format.js';

const props = defineProps({
  scan: { type: Object, default: null },
  loading: Boolean,
  error: { type: String, default: '' },
  selectedCount: { type: Number, default: 0 },
  busy: Boolean,
  importing: Boolean,
  deleting: Boolean,
  configuredDir: { type: String, default: '' }
});

const emit = defineEmits([
  'refresh',
  'import-selected',
  'delete-selected',
  'edit-directory',
  'go-images',
  'selection-change'
]);

const tableRef = ref(null);

const files = computed(() => props.scan?.files || []);
const directory = computed(() => props.scan?.offlineDir || props.configuredDir || '未配置');
const summary = computed(() => {
  if (!props.scan) {
    return props.loading ? '正在读取' : '未读取';
  }
  return `${files.value.length} 个文件 · 共 ${formatSize(props.scan.totalBytes)}`;
});
const emptyTitle = computed(() => {
  if (props.loading) {
    return '正在读取离线包';
  }
  return props.scan?.exists === false ? '离线包目录尚未创建' : '还没有离线包';
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

.selection-row,
.dir-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.selection-row {
  gap: 12px;
  padding: 12px 14px;
  margin-bottom: 14px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
}

.dir-row {
  margin-bottom: 14px;
}

.selection-hint {
  font-size: 12px;
  color: var(--text-secondary);
  margin-right: 4px;
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

.error-alert {
  margin-bottom: 14px;
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

.ref-line {
  display: block;
  line-height: 1.7;
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
