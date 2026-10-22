<template>
  <div v-if="candidate" class="prepare-panel">
    <div class="selected-package">
      <span class="package-icon"><el-icon><Box /></el-icon></span>
      <div>
        <h4>{{ candidate.label }}</h4>
        <p v-if="candidate.version" class="field-hint">版本：{{ versionLabel(candidate.version) }}</p>
        <p v-if="candidate.path" class="path-text">{{ candidate.path }}</p>
      </div>
    </div>
    <p class="prepare-hint">{{ hint }}</p>
    <el-button
      type="primary"
      :loading="preparing"
      :disabled="controlsDisabled || !desktopAvailable || Boolean(preparedBundle)"
      @click="emit('prepare')"
    >
      {{ preparedBundle ? '已校验完成' : label }}
    </el-button>
    <div v-if="preparing || progress.stage" class="prepare-progress" role="status" aria-live="polite">
      <el-progress
        v-if="progress.total > 0 || preparedBundle"
        :percentage="progress.percent"
        :status="preparedBundle ? 'success' : undefined"
      />
      <p class="field-hint">{{ progress.stage }}</p>
    </div>
    <el-alert
      v-if="preparedBundle"
      title="交付包已准备就绪"
      :description="`已完成交付包校验，共 ${preparedBundle.files} 个文件，可继续读取部署清单。`"
      type="success"
      :closable="false"
      show-icon
    />
    <p v-if="preparedBundle" class="prepared-directory">交付包目录：{{ preparedBundle.dir }}</p>
  </div>
  <el-empty v-else description="请先返回上一步选择交付包" :image-size="72" />
</template>

<script setup>
import { Box } from '@element-plus/icons-vue';
import { versionLabel } from '../domain/bundle-selection.js';

defineProps({
  candidate: { type: Object, default: null },
  label: { type: String, default: '' },
  hint: { type: String, default: '' },
  preparing: { type: Boolean, default: false },
  controlsDisabled: { type: Boolean, default: false },
  desktopAvailable: { type: Boolean, default: false },
  preparedBundle: { type: Object, default: null },
  progress: { type: Object, required: true }
});

const emit = defineEmits(['prepare']);
</script>

<style scoped>
.prepare-panel {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.selected-package {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.selected-package h4 {
  font-size: 14px;
  line-height: 1.5;
}

.selected-package .field-hint,
.selected-package .path-text {
  margin-top: 6px;
}

.package-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 42px;
  height: 42px;
  flex: none;
  border-radius: var(--radius-sm);
  background: var(--primary-soft);
  color: var(--primary-color);
  font-size: 22px;
}

.prepare-hint {
  color: var(--text-muted);
  font-size: 13px;
  line-height: 1.6;
}

.prepare-panel > .el-button {
  align-self: flex-start;
}

.field-hint,
.path-text,
.prepared-directory {
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}

.path-text,
.prepared-directory {
  overflow-wrap: anywhere;
}

.prepare-progress .field-hint {
  margin-top: 8px;
}
</style>
