<template>
  <div class="card-surface remote-card">
    <div class="card-title-bar">
      <h3>离线包来源</h3>
      <span class="hint">{{ countText }}</span>
    </div>
    <div class="summary-row">
      <span class="summary-text">保存目录</span>
      <code class="path-text">{{ dir || '正在读取' }}</code>
      <el-button size="small" :loading="loading" @click="$emit('refresh')">
        <el-icon><Refresh /></el-icon>
        <span>刷新离线包</span>
      </el-button>
    </div>
    <div class="field-hint">
      这里列出设置中保存目录下的离线包。没有离线包时，请先在镜像列表勾选镜像并导出。
    </div>
  </div>
</template>

<script setup>
import { Refresh } from '@element-plus/icons-vue';

defineProps({
  dir: {
    type: String,
    default: ''
  },
  countText: {
    type: String,
    default: ''
  },
  loading: {
    type: Boolean,
    default: false
  }
});

defineEmits(['refresh']);
</script>

<style scoped>
.remote-card {
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

.field-hint {
  width: 100%;
  margin-top: 6px;
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-muted);
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
</style>
