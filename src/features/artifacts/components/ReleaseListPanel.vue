<template>
  <div class="card-surface versions-card">
    <div class="card-title-bar">
      <div>
        <h3>制品列表</h3>
        <p class="card-subtitle">按构建时间倒序排列，最新版本置顶；打开详情可查看该版本的交付包链路</p>
      </div>
      <el-button v-if="releases.length" link :loading="loading" @click="emit('refresh')">
        <el-icon><Refresh /></el-icon>
        <span>刷新列表</span>
      </el-button>
    </div>

    <el-alert v-if="error" type="warning" :closable="false" show-icon class="versions-alert">
      <template #title>{{ error }}</template>
      <div class="alert-actions">
        <span v-if="releases.length">已显示本机缓存的版本，连接恢复后刷新即可获取最新构建。</span>
        <span v-else>请检查网络或制品源配置后重试。</span>
        <el-button size="small" @click="emit('refresh')">重试</el-button>
        <el-button size="small" @click="emit('go-settings')">前往设置</el-button>
      </div>
    </el-alert>

    <el-table
      v-if="releases.length"
      :data="releases"
      :row-key="releaseKey"
      size="small"
      class="versions-table"
    >
      <el-table-column label="版本" min-width="150">
        <template #default="{ row }">
          <div class="version-cell">
            <div class="version-title">
              <span class="version-number">{{ versionText(row) }}</span>
              <el-tag v-if="isLatest(row)" size="small" type="primary" effect="plain">最新</el-tag>
            </div>
          </div>
        </template>
      </el-table-column>
      <el-table-column label="构建信息" min-width="180">
        <template #default="{ row }">
          <div v-if="row.runId" class="stacked-cell">
            <div class="commit-line">
              <code>{{ row.commitShort || '-' }}</code>
              <span class="commit-title">{{ row.commitTitle || '无提交说明' }}</span>
            </div>
            <span class="subtle">
              运行 #{{ row.runId }}<template v-if="row.branch"> · 分支 {{ row.branch }}</template> ·
              {{ artifactSizeText(row.sizeBytes) }}
            </span>
          </div>
          <div v-else-if="row.commitShort || row.commit" class="stacked-cell">
            <div class="commit-line">
              <code>{{ row.commitShort || row.commit }}</code>
            </div>
            <span class="subtle">
              本机缓存版本<template v-if="row.branch"> · 分支 {{ row.branch }}</template> ·
              {{ artifactSizeText(row.sizeBytes) }}
            </span>
          </div>
          <span v-else class="subtle">{{ sourceType === 'url' ? '直接下载源，无构建信息' : '本机缓存版本' }}</span>
        </template>
      </el-table-column>
      <el-table-column label="构建时间" width="145">
        <template #default="{ row }">{{ artifactTimeText(row.builtAt) }}</template>
      </el-table-column>
      <el-table-column label="操作" width="200">
        <template #default="{ row }">
          <el-button link @click="emit('show-detail', row)">详情</el-button>
          <el-button v-if="!row.localDir && row.downloadUrl" link type="primary" @click="emit('download', row)">下载</el-button>
          <el-button v-if="row.localDir && row.downloadUrl" link @click="emit('download', row)">重新下载</el-button>
          <el-button v-if="row.runUrl" link @click="emit('open-run', row)">运行详情</el-button>
        </template>
      </el-table-column>
    </el-table>

    <div v-else-if="loading" class="versions-empty">
      <span class="empty-icon compact"><el-icon><Box /></el-icon></span>
      <h4>正在读取版本</h4>
      <p>正在获取该制品源的最新构建与历史记录。</p>
    </div>

    <div v-else-if="!error" class="versions-empty">
      <span class="empty-icon compact"><el-icon><Box /></el-icon></span>
      <h4>当前未找到可用制品</h4>
      <p>{{ hasMore ? '可以继续加载更早的构建记录。' : '请确认流水线已成功构建，并上传了对应制品。' }}</p>
    </div>

    <div v-if="(releases.length || hasMore) && !error" class="list-footer">
      <el-button v-if="hasMore" :loading="loadingMore" @click="emit('load-more')">加载更早版本</el-button>
      <span v-else class="subtle">已显示全部可加载版本</span>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue';
import { Box, Refresh } from '@element-plus/icons-vue';
import {
  artifactSizeText,
  artifactTimeText,
  releaseKey,
  versionText
} from '../domain/releases.js';

const props = defineProps({
  releases: { type: Array, default: () => [] },
  loading: Boolean,
  error: { type: String, default: '' },
  hasMore: Boolean,
  loadingMore: Boolean,
  sourceType: { type: String, default: '' }
});

const emit = defineEmits(['refresh', 'go-settings', 'load-more', 'show-detail', 'download', 'open-run']);

const latestRelease = computed(() => props.releases[0] || null);

const isLatest = (item) => Boolean(item) && latestRelease.value === item;
</script>

<style scoped>
.versions-card {
  padding: 22px 24px;
}

.card-title-bar {
  align-items: flex-start;
}

.versions-table {
  margin-top: 4px;
}

.versions-table :deep(.el-button + .el-button) {
  margin-left: 8px;
}

.versions-table :deep(.cell) {
  white-space: nowrap;
}

.version-cell,
.stacked-cell {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.version-title,
.commit-line {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.version-title {
  flex-wrap: wrap;
}

.version-number {
  color: var(--text-main);
  font-size: 14px;
  font-weight: 650;
}

.commit-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
  color: var(--text-secondary);
}

.subtle {
  color: var(--text-muted);
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.versions-alert {
  margin-bottom: 14px;
}

.alert-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 6px;
}

.versions-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 36px 24px;
  text-align: center;
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

.empty-icon.compact {
  width: 42px;
  height: 42px;
  margin-bottom: 0;
  border-radius: 12px;
  font-size: 20px;
}

.versions-empty h4 {
  margin-top: 12px;
  color: var(--text-main);
  font-size: 14px;
  font-weight: 600;
}

.versions-empty p {
  max-width: 420px;
  margin: 7px 0 14px;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}

.list-footer {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 46px;
  margin-top: 10px;
  padding-top: 12px;
  border-top: 1px solid var(--border-color);
  color: var(--text-muted);
  font-size: 12px;
}
</style>
