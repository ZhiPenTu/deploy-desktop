<template>
  <el-table
    :data="images"
    size="small"
    :row-class-name="rowClass"
    :empty-text="filter ? '没有符合此状态的镜像' : '此配置没有需要检查的镜像'"
  >
    <el-table-column label="镜像" min-width="320">
      <template #default="{ row }">
        <div class="image-cell">
          <code>{{ row.imageRef }}</code>
        </div>
      </template>
    </el-table-column>
    <el-table-column label="服务" min-width="160">
      <template #default="{ row }">
        <el-tag
          v-for="service in row.services"
          :key="service"
          size="small"
          effect="plain"
          class="service-tag"
        >
          {{ service }}
        </el-tag>
      </template>
    </el-table-column>
    <el-table-column label="本地镜像包" min-width="240">
      <template #default="{ row }">
        <div class="image-cell">
          <code v-if="row.localFile" class="file-name">{{ row.localFile }}</code>
          <span v-else :class="row.dockerPresent || imageCheck === 'unavailable' ? 'muted' : 'missing-text'">
            {{ row.dockerPresent ? '未保存离线包' : '未找到离线包' }}
          </span>
          <span v-if="row.localFile" class="image-sub">
            {{ composeSizeText(row.localBytes) }} · {{ sourceLabel(row.matchSource) }}
          </span>
        </div>
      </template>
    </el-table-column>
    <el-table-column :label="dockerColumnLabel(imageCheck)" width="130">
      <template #default="{ row }">
        <el-tag size="small" :type="dockerTone(row, imageCheck)" effect="plain">
          {{ dockerLabel(row, imageCheck) }}
        </el-tag>
      </template>
    </el-table-column>
    <el-table-column label="状态" width="110">
      <template #default="{ row }">
        <el-tag size="small" :type="statusTone(row.status)" effect="light">
          {{ statusLabel(row.status, available) }}
        </el-tag>
      </template>
    </el-table-column>
  </el-table>

  <ul v-if="issues.length" class="issue-list">
    <li v-for="issue in issues" :key="issue">{{ issue }}</li>
  </ul>
</template>

<script setup>
import { computed } from 'vue';
import {
  composeSizeText,
  dockerColumnLabel,
  dockerLabel,
  dockerTone,
  imageAvailableLabel,
  sourceLabel,
  statusLabel,
  statusTone
} from '../domain/compose.js';

const props = defineProps({
  images: { type: Array, default: () => [] },
  filter: { type: String, default: '' },
  imageCheck: { type: String, default: '' },
  issues: { type: Array, default: () => [] }
});

const available = computed(() => imageAvailableLabel(props.imageCheck));

function rowClass({ row }) {
  return `status-${row.status}`;
}
</script>

<style scoped>
.image-cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.image-cell code {
  font-size: 12px;
  color: var(--text-main);
  word-break: break-all;
}

.image-sub {
  font-size: 11px;
  color: var(--text-faint);
}

.file-name {
  color: var(--primary-color);
}

.missing-text {
  font-size: 12px;
  color: var(--danger-color);
}

.service-tag {
  margin: 2px 4px 2px 0;
}

.muted {
  color: var(--text-faint);
  font-size: 12px;
}

.issue-list {
  margin: 10px 0 0;
  padding-left: 18px;
  font-size: 11.5px;
  color: var(--warning-color);
  display: flex;
  flex-direction: column;
  gap: 4px;
}

:deep(.el-table .status-missing td) {
  background: var(--danger-soft);
}

:deep(.el-table .status-to_import td) {
  background: var(--warning-soft);
}
</style>
