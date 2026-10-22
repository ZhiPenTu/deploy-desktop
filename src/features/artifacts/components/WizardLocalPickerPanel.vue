<template>
  <section class="artifact-section">
    <div class="section-heading">
      <div>
        <h4>选择本机制品</h4>
        <p>选择 .tgz、.tar.gz 交付包，或已解压的交付包目录。</p>
      </div>
    </div>
    <div class="local-actions">
      <el-button :disabled="controlsDisabled || !desktopAvailable" @click="emit('pick', false)">
        <el-icon><FolderOpened /></el-icon>
        <span>选择压缩包</span>
      </el-button>
      <el-button :disabled="controlsDisabled || !desktopAvailable" @click="emit('pick', true)">
        选择已解压目录
      </el-button>
      <el-button
        v-if="currentBundleDir"
        :disabled="controlsDisabled || !desktopAvailable"
        @click="emit('choose-current')"
      >
        使用当前交付包目录
      </el-button>
    </div>
  </section>
</template>

<script setup>
import { FolderOpened } from '@element-plus/icons-vue';

defineProps({
  currentBundleDir: { type: String, default: '' },
  controlsDisabled: { type: Boolean, default: false },
  desktopAvailable: { type: Boolean, default: false }
});

const emit = defineEmits(['pick', 'choose-current']);
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

.local-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.local-actions :deep(.el-button + .el-button) {
  margin-left: 0;
}

@media (max-width: 760px) {
  .artifact-section {
    padding: 16px;
  }
}
</style>
