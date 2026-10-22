<template>
  <div class="card-surface remote-card">
    <div class="card-title-bar">
      <h3>执行结果</h3>
      <span class="hint">{{ title }}</span>
    </div>
    <pre class="output-log">{{ log }}</pre>
  </div>
</template>

<script setup>
defineProps({
  log: {
    type: String,
    required: true
  },
  title: {
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

.output-log {
  max-height: 240px;
  overflow: auto;
  margin: 0;
  padding: 14px;
  border-radius: var(--radius-sm);
  background: #0b1120;
  border: 1px solid #1e293b;
  color: #cbd5e1;
  font-size: 12px;
  line-height: 1.6;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: pre-wrap;
}
</style>
