<template>
  <div class="card-surface remote-card">
    <div class="card-title-bar">
      <h3>执行进度</h3>
      <span class="hint">{{ message }}</span>
    </div>
    <el-progress :percentage="percent" class="progress-bar" />
    <div class="progress-list">
      <div
        v-for="(item, index) in progress.slice(-8)"
        :key="`${item.file_name}-${index}`"
        class="progress-row"
      >
        <span class="progress-percent">{{ item.percent }}%</span>
        <span class="progress-text">{{ item.message }}</span>
      </div>
    </div>
  </div>
</template>

<script setup>
defineProps({
  progress: {
    type: Array,
    required: true
  },
  percent: {
    type: Number,
    default: 0
  },
  message: {
    type: String,
    default: ''
  }
});
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

.progress-bar {
  margin-bottom: 12px;
}

.progress-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.progress-row {
  display: flex;
  gap: 10px;
  align-items: baseline;
  font-size: 11px;
  color: var(--text-muted);
}

.progress-percent {
  width: 42px;
  text-align: right;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  color: var(--text-secondary);
}

.progress-text {
  overflow-wrap: anywhere;
}
</style>
