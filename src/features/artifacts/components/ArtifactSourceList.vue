<template>
  <div v-if="!loading && !sources.length" class="empty-panel card-surface">
    <span class="empty-icon"><el-icon><Box /></el-icon></span>
    <h3>还没有可用的制品源</h3>
    <p>在设置的“制品设置”中添加云效流水线或直链下载来源后，即可在这里查看版本并下载交付包。</p>
    <div class="empty-actions">
      <el-button type="primary" @click="emit('go-settings')">前往设置</el-button>
      <el-button :loading="loading" @click="emit('refresh')">
        <el-icon><Refresh /></el-icon>
        <span>刷新</span>
      </el-button>
    </div>
    <span class="empty-hint">令牌仅保存在本机，供当前用户读取</span>
  </div>

  <div v-else class="card-surface source-card">
    <div class="card-title-bar">
      <div>
        <h3>制品源</h3>
        <p class="card-subtitle">共 {{ sources.length }} 个来源，点击名称或“查看制品”进入版本列表</p>
      </div>
      <span class="hint">令牌仅保存在本机，供当前用户读取</span>
    </div>
    <el-table :data="sources" size="small" class="source-table">
      <el-table-column label="名称" min-width="160">
        <template #default="{ row }">
          <div class="name-cell">
            <el-button link class="source-name" @click="emit('open-source', row)">
              {{ row.name }}
            </el-button>
            <el-tag size="small" effect="plain">
              {{ row.type === 'url' ? '直接下载' : '云效流水线' }}
            </el-tag>
          </div>
        </template>
      </el-table-column>
      <el-table-column label="连接" min-width="130">
        <template #default="{ row }">
          <div v-if="row.type !== 'url'" class="stacked-cell">
            <span>流水线 #{{ row.pipelineId }}</span>
            <span class="subtle">组织 {{ row.orgId }}</span>
          </div>
          <span v-else class="subtle">直接下载地址</span>
        </template>
      </el-table-column>
      <el-table-column label="制品名" min-width="100">
        <template #default="{ row }">
          <code>{{ row.artifactName }}</code>
        </template>
      </el-table-column>
      <el-table-column label="本地版本" min-width="140">
        <template #default="{ row }">
          <span :class="{ muted: !row.lastVersion }">{{ row.lastVersion || '尚未下载' }}</span>
        </template>
      </el-table-column>
      <el-table-column label="操作" width="105">
        <template #default="{ row }">
          <el-button link type="primary" @click="emit('open-source', row)">查看制品</el-button>
        </template>
      </el-table-column>
    </el-table>
  </div>
</template>

<script setup>
import { Box, Refresh } from '@element-plus/icons-vue';

defineProps({
  sources: { type: Array, default: () => [] },
  loading: Boolean
});

const emit = defineEmits(['open-source', 'refresh', 'go-settings']);
</script>

<style scoped>
.source-card {
  padding: 22px 24px;
}

.card-title-bar {
  align-items: flex-start;
}

.source-table {
  margin-top: 4px;
}

.source-table :deep(.el-button + .el-button) {
  margin-left: 8px;
}

.source-table :deep(.cell) {
  white-space: nowrap;
}

.name-cell {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.source-name {
  min-width: 0;
  max-width: 230px;
  justify-content: flex-start;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}

.stacked-cell {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.subtle {
  color: var(--text-muted);
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.muted {
  color: var(--text-muted);
  word-break: break-all;
}

.empty-panel {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  padding: 56px 32px 48px;
  background:
    radial-gradient(420px 180px at 50% 0%, rgba(51, 92, 255, 0.06), transparent 70%),
    var(--panel-bg);
}

.empty-icon {
  width: 52px;
  height: 52px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 14px;
  background: var(--primary-soft);
  color: var(--primary-color);
  font-size: 24px;
  margin-bottom: 18px;
}

.empty-panel h3 {
  font-size: 16px;
  font-weight: 600;
  color: var(--text-main);
}

.empty-panel p {
  max-width: 440px;
  margin-top: 8px;
  font-size: 12.5px;
  line-height: 1.65;
  color: var(--text-muted);
}

.empty-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 22px;
}

.empty-hint {
  margin-top: 16px;
  font-size: 11px;
  color: var(--text-faint);
}
</style>
