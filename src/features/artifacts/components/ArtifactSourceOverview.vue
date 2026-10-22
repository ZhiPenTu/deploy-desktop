<template>
  <div class="source-overview card-surface">
    <div class="overview-main">
      <div class="overview-title-line">
        <h3>{{ source.name }}</h3>
        <el-tag size="small" effect="plain">
          {{ source.type === 'url' ? '直接下载' : '云效流水线' }}
        </el-tag>
      </div>
      <div class="overview-meta">
        <span v-if="source.type === 'url'">来源：直接下载地址</span>
        <span v-else>来源：云效流水线 #{{ source.pipelineId }}</span>
        <span>制品名：<code>{{ source.artifactName }}</code></span>
        <span v-if="source.lastVersion">本地版本：{{ source.lastVersion }}</span>
      </div>
    </div>
    <div class="overview-actions">
      <el-select
        :model-value="activeSourceId"
        aria-label="选择制品源"
        class="source-switcher"
        @change="(id) => emit('switch-source', id)"
      >
        <el-option v-for="item in sources" :key="item.id" :label="item.name" :value="item.id" />
      </el-select>
      <el-button
        v-if="source.lastBundleDir"
        type="warning"
        plain
        :loading="removing"
        @click="emit('remove-local', source)"
      >
        清理本地缓存
      </el-button>
    </div>
    <div class="overview-stats">
      <div class="overview-stat">
        <span class="stat-label">最新版本</span>
        <strong>{{ latestVersionText }}</strong>
      </div>
      <div class="overview-stat">
        <span class="stat-label">最近构建</span>
        <strong>{{ latestBuiltText }}</strong>
      </div>
      <div class="overview-stat">
        <span class="stat-label">已加载制品</span>
        <strong>{{ loadedVersionText }}</strong>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue';
import { artifactTimeText, versionText } from '../domain/releases.js';

const props = defineProps({
  source: { type: Object, required: true },
  sources: { type: Array, default: () => [] },
  activeSourceId: { type: String, default: '' },
  removing: Boolean,
  releases: { type: Array, default: () => [] },
  releasesLoading: Boolean,
  hasMore: Boolean
});

const emit = defineEmits(['switch-source', 'remove-local']);

const latestRelease = computed(() => props.releases[0] || null);
const latestVersionText = computed(() => {
  if (props.releasesLoading && !latestRelease.value) return '读取中';
  return latestRelease.value ? versionText(latestRelease.value) : '-';
});
const latestBuiltText = computed(() => {
  if (props.releasesLoading && !latestRelease.value) return '读取中';
  return latestRelease.value ? artifactTimeText(latestRelease.value.builtAt) : '-';
});
const loadedVersionText = computed(() => {
  if (props.releasesLoading && !props.releases.length) return '读取中';
  return `${props.releases.length}${props.hasMore ? '+' : ''}`;
});
</script>

<style scoped>
.source-overview {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 18px 24px;
  padding: 20px;
}

.overview-title-line,
.overview-meta,
.overview-actions,
.overview-stats {
  display: flex;
  align-items: center;
}

.overview-title-line {
  gap: 10px;
}

.overview-title-line h3 {
  font-size: 17px;
  font-weight: 650;
}

.overview-meta {
  flex-wrap: wrap;
  gap: 6px 18px;
  margin-top: 9px;
  color: var(--text-muted);
  font-size: 12px;
}

.overview-actions {
  gap: 8px;
  align-self: start;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.source-switcher {
  width: 200px;
}

.overview-stats {
  grid-column: 1 / -1;
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 14px;
  padding-top: 18px;
  border-top: 1px solid var(--border-subtle);
}

.overview-stat {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 12px 16px;
  background: var(--panel-subtle);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  min-width: 0;
}

.stat-label {
  color: var(--text-muted);
  font-size: 11px;
}

.overview-stat strong {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-main);
  font-size: 13px;
  font-weight: 600;
}

@media (max-width: 980px) {
  .source-overview {
    grid-template-columns: 1fr;
  }

  .overview-actions {
    align-self: stretch;
  }

  .overview-stats {
    flex-wrap: wrap;
  }

  .overview-stat {
    flex-basis: 180px;
  }
}
</style>
