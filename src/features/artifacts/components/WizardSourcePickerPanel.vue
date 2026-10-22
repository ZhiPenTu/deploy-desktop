<template>
  <section class="artifact-section">
    <div class="section-heading">
      <div>
        <h4>从制品源选择</h4>
        <p>使用已下载的交付包，或检查最新可用版本。</p>
      </div>
      <el-button link type="primary" :disabled="controlsDisabled" @click="emit('go-artifacts')">
        管理制品源
      </el-button>
    </div>

    <div class="source-controls">
      <el-select
        :model-value="sourceId"
        placeholder="请选择制品源"
        :disabled="controlsDisabled || !desktopAvailable"
        aria-label="制品源"
        @update:model-value="(value) => emit('update:source-id', value)"
      >
        <el-option v-for="source in sources" :key="source.id" :label="source.name" :value="source.id" />
      </el-select>
      <el-button
        :loading="operation === 'sources'"
        :disabled="controlsDisabled || !desktopAvailable"
        @click="emit('refresh')"
      >
        <el-icon><Refresh /></el-icon>
        <span>刷新</span>
      </el-button>
    </div>

    <p v-if="desktopAvailable && !sources.length && !busy" class="field-hint">
      尚未配置制品源，请先在设置的“制品设置”中添加，或选择本机制品。
    </p>
    <div v-if="selectedSource" class="source-actions">
      <el-button
        :disabled="controlsDisabled || !selectedSource.lastBundleDir"
        @click="emit('choose-downloaded')"
      >
        使用已下载包{{ selectedSource.lastVersion && selectedSource.lastVersion !== 'manual' ? `（${selectedSource.lastVersion}）` : '' }}
      </el-button>
      <el-button
        :loading="operation === 'release'"
        :disabled="controlsDisabled"
        @click="emit('check-latest')"
      >
        检查最新版本
      </el-button>
      <el-button
        :loading="operation === 'versions'"
        :disabled="controlsDisabled || !desktopAvailable"
        @click="emit('toggle-versions')"
      >
        {{ versionsVisible ? '收起版本列表' : '选择版本' }}
      </el-button>
      <span v-if="!selectedSource.lastBundleDir" class="field-hint">该制品源尚无已下载的交付包</span>
    </div>

    <div v-if="versionsVisible" class="version-list">
      <p v-if="!versions.length && operation !== 'versions'" class="field-hint">
        该制品源暂无可用版本。
      </p>
      <div v-for="item in versions" :key="releaseKey(item)" class="version-row">
        <div class="version-info">
          <div class="summary-line">
            <strong>{{ versionLabel(item.version) }}</strong>
            <el-tag v-if="item.commitShort" size="small" type="info" effect="plain">{{ item.commitShort }}</el-tag>
            <el-tag v-if="item.localDir" size="small" type="success" effect="plain">已下载</el-tag>
          </div>
          <p class="field-hint">
            构建时间：{{ timeLabel(item.builtAt) }}
            <template v-if="item.sizeBytes">，大小：{{ sizeLabel(item.sizeBytes) }}</template>
          </p>
        </div>
        <el-button
          size="small"
          :type="isChosen(item) ? 'primary' : ''"
          :disabled="controlsDisabled || isChosen(item)"
          @click="emit('choose-version', item)"
        >
          {{ isChosen(item) ? '已选择' : item.localDir ? '使用此版本' : '下载此版本' }}
        </el-button>
      </div>
      <el-button
        v-if="versionsHasMore"
        size="small"
        :loading="operation === 'versions'"
        :disabled="controlsDisabled"
        @click="emit('load-more')"
      >
        加载更多版本
      </el-button>
    </div>

    <div v-if="release" class="release-summary">
      <div class="summary-line">
        <span class="summary-label">最新版本</span>
        <strong>{{ versionLabel(release.version) }}</strong>
        <el-tag v-if="release.commitShort" size="small" type="info" effect="plain">{{ release.commitShort }}</el-tag>
      </div>
      <p v-if="release.sizeBytes" class="field-hint">交付包大小：{{ sizeLabel(release.sizeBytes) }}</p>
      <el-button
        type="primary"
        :disabled="controlsDisabled || isChosen(release)"
        @click="emit('choose-release')"
      >
        {{ isChosen(release) ? '已选择' : '选择此版本' }}
      </el-button>
    </div>
  </section>
</template>

<script setup>
import { Refresh } from '@element-plus/icons-vue';
import { releaseKey } from '../domain/releases.js';
import { isChosenVersion, sizeLabel, timeLabel, versionLabel } from '../domain/bundle-selection.js';

const props = defineProps({
  sources: { type: Array, default: () => [] },
  sourceId: { type: String, default: '' },
  selectedSource: { type: Object, default: null },
  versions: { type: Array, default: () => [] },
  versionsVisible: { type: Boolean, default: false },
  versionsHasMore: { type: Boolean, default: false },
  release: { type: Object, default: null },
  candidate: { type: Object, default: null },
  operation: { type: String, default: '' },
  busy: { type: Boolean, default: false },
  controlsDisabled: { type: Boolean, default: false },
  desktopAvailable: { type: Boolean, default: false }
});

const emit = defineEmits([
  'update:source-id',
  'refresh',
  'go-artifacts',
  'choose-downloaded',
  'check-latest',
  'toggle-versions',
  'load-more',
  'choose-version',
  'choose-release'
]);

function isChosen(item) {
  return isChosenVersion(props.candidate, props.sourceId, item);
}
</script>

<style scoped>
.artifact-section {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 20px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--panel-bg);
}

.section-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.section-heading h4 {
  font-size: 14px;
  line-height: 1.5;
}

.section-heading p {
  margin-top: 4px;
  color: var(--text-muted);
  font-size: 13px;
  line-height: 1.6;
}

.source-controls,
.source-actions,
.summary-line {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.source-controls .el-select {
  flex: 1;
  min-width: 180px;
}

.source-actions :deep(.el-button + .el-button) {
  margin-left: 0;
}


.version-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 16px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
}

.version-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 0;
  border-bottom: 1px solid var(--border-color);
}

.version-row:last-of-type {
  border-bottom: none;
}

.version-info {
  min-width: 0;
}

.version-list > .el-button {
  align-self: flex-start;
}

.release-summary {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 12px;
  padding: 16px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
}

.summary-label {
  color: var(--text-muted);
}

.field-hint {
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}

@media (max-width: 760px) {
  .artifact-section {
    padding: 16px;
  }

  .section-heading {
    flex-wrap: wrap;
  }
}
</style>
