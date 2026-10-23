<template>
  <div class="card-surface remote-card">
    <div class="card-title-bar">
      <div>
        <h3>分发目标</h3>
        <p class="card-subtitle">选择分发主机，检测该主机上的镜像包与 Docker 状态</p>
      </div>
      <span v-if="remote" class="hint">
        {{ remote.exists ? `${remote.files.length} 个文件 · ${formatSize(remote.total_bytes)}` : '镜像目录未就绪' }}
      </span>
    </div>

    <el-form label-width="110px">
      <el-form-item label="分发主机">
        <el-select
          v-model="hostId"
          class="host-select"
          placeholder="请选择分发主机"
          :disabled="disabled"
        >
          <el-option
            v-for="host in hosts"
            :key="host.id"
            :value="host.id"
            :label="`${host.name} · ${host.address}`"
          />
        </el-select>
        <div class="field-hint">上传、导入与删除只作用于所选主机，不影响项目与部署向导使用的目标主机。</div>
      </el-form-item>
      <el-form-item label="目标机目录">
        <div class="dir-input-row">
          <el-input v-model="dirPath" placeholder="/home/deploy/bundle" />
          <el-button :loading="inspecting" @click="$emit('inspect')">
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
      v-if="error"
      type="warning"
      :closable="false"
      show-icon
      :title="error"
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
      v-if="remote && rows.length"
      :data="rows"
      size="small"
      class="image-table"
      @selection-change="$emit('selection-change', $event)"
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
        <template #default="{ row }">{{ row.present ? formatTimestamp(row.modified_ms) : '-' }}</template>
      </el-table-column>
    </el-table>

    <div v-else class="empty-state">
      <span class="icon-tile neutral"><el-icon><Box /></el-icon></span>
      <span class="empty-hint">
        {{ emptyHint }}
      </span>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue';
import { Box, Refresh } from '@element-plus/icons-vue';
import { formatSize, formatTimestamp } from '../../../shared/format.js';

const props = defineProps({
  hosts: {
    type: Array,
    default: () => []
  },
  remote: {
    type: Object,
    default: null
  },
  error: {
    type: String,
    default: ''
  },
  rows: {
    type: Array,
    default: () => []
  },
  inspecting: {
    type: Boolean,
    default: false
  },
  disabled: {
    type: Boolean,
    default: false
  }
});

defineEmits(['inspect', 'selection-change']);

const dirPath = defineModel('dirPath', { type: String, default: '' });
const hostId = defineModel('hostId', { type: String, default: '' });

const isRowSelectable = (row) => row.present && row.known;

const emptyHint = computed(() => {
  if (!hostId.value) return '请先选择分发主机，再检测目标机镜像包';
  if (!props.remote) return '点击“检测目标机”读取镜像包与 Docker 镜像状态';
  return props.remote.exists ? '目标机镜像目录为空' : '目标机镜像目录未就绪，请先上传离线包';
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

.host-select {
  width: 320px;
  max-width: 100%;
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
</style>
