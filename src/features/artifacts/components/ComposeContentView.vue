<template>
  <div class="code-view" :class="{ single: standalone }">
    <div
      v-for="line in lines"
      :key="line.no"
      class="code-line"
      :class="{ highlight: line.highlight }"
    >
      <span class="line-no">{{ line.no }}</span>
      <span class="line-text">{{ line.text }}</span>
    </div>
  </div>
  <p v-if="showTruncate && truncated" class="truncate-hint">
    内容较长，仅显示前 {{ maxLines }} 行，完整文件请直接打开 {{ file }}
  </p>
</template>

<script setup>
import { computed } from 'vue';
import { contentLines } from '../domain/compose.js';

const props = defineProps({
  content: { type: String, default: '' },
  file: { type: String, default: '' },
  maxLines: { type: Number, default: 1500 },
  standalone: { type: Boolean, default: false },
  showTruncate: { type: Boolean, default: true }
});

const parsed = computed(() => contentLines(props.content, props.maxLines));
const lines = computed(() => parsed.value.lines);
const truncated = computed(() => parsed.value.truncated);
</script>

<style scoped>
.code-view {
  max-height: 52vh;
  overflow: auto;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: #0b1220;
  padding: 10px 0;
  font-family: 'SFMono-Regular', Menlo, Consolas, monospace;
}

.code-view.single {
  max-height: 62vh;
}

.code-line {
  display: flex;
  gap: 12px;
  padding: 0 12px;
  font-size: 12px;
  line-height: 1.6;
  color: #d7dde8;
  white-space: pre;
}

.code-line.highlight {
  background: rgba(51, 92, 255, 0.16);
  color: #ffffff;
}

.line-no {
  flex: none;
  width: 42px;
  text-align: right;
  color: #5b6b85;
  user-select: none;
}

.line-text {
  flex: 1;
  min-width: 0;
}

.truncate-hint {
  margin-top: 8px;
  font-size: 11.5px;
  color: var(--text-faint);
}
</style>
