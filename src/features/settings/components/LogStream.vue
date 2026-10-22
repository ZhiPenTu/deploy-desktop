<template>
  <div v-loading="loading" class="log-stream">
    <template v-for="group in grouped" :key="group.date">
      <div class="log-date">{{ group.label }}</div>
      <div
        v-for="row in group.rows"
        :key="row.key"
        class="log-item"
        :class="`is-${row.tone}`"
      >
        <span class="log-marker" />
        <span class="log-time">{{ row.clock }}</span>
        <span class="log-level">{{ row.levelText }}</span>
        <span class="log-target">{{ row.target }}</span>
        <span
          class="log-text"
          :class="{ 'is-open': expanded.has(row.key) }"
          @click="emit('toggle', row.key)"
        >{{ row.message }}</span>
      </div>
    </template>

    <div v-if="!loading && !rows.length" class="log-empty">
      <span class="empty-icon"><el-icon><Document /></el-icon></span>
      <span class="empty-title">{{ emptyTitle }}</span>
      <span class="empty-hint">{{ emptyHint }}</span>
    </div>
  </div>
</template>

<script setup>
import { Document } from '@element-plus/icons-vue';

defineProps({
  grouped: { type: Array, default: () => [] },
  rows: { type: Array, default: () => [] },
  loading: { type: Boolean, default: false },
  expanded: { type: Object, default: () => new Set() },
  emptyTitle: { type: String, default: '' },
  emptyHint: { type: String, default: '' }
});

const emit = defineEmits(['toggle']);
</script>

<style scoped>
.log-stream {
  display: flex;
  flex-direction: column;
  max-height: 560px;
  min-height: 220px;
  overflow-y: auto;
}

.log-date {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: 8px 18px;
  font-size: 11.5px;
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--text-faint);
  background: var(--panel-subtle);
  border-bottom: 1px solid var(--border-color);
}

.log-item {
  display: grid;
  grid-template-columns: 4px 96px 52px 120px minmax(0, 1fr);
  align-items: baseline;
  gap: 12px;
  padding: 9px 18px 9px 0;
  border-bottom: 1px solid rgba(230, 233, 239, 0.7);
  transition: background 0.12s ease;
}

.log-item:hover {
  background: var(--panel-subtle);
}

.log-marker {
  align-self: stretch;
  width: 4px;
  border-radius: 0 3px 3px 0;
  background: var(--border-strong);
}

.log-item.is-info .log-marker {
  background: var(--success-color);
}

.log-item.is-warn .log-marker {
  background: var(--warning-color);
}

.log-item.is-error .log-marker {
  background: var(--danger-color);
}

.log-item.is-debug .log-marker {
  background: var(--text-faint);
}

.log-time {
  font-family: var(--el-font-family-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-size: 12px;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}

.log-level {
  font-size: 12px;
  font-weight: 600;
}

.log-item.is-info .log-level {
  color: var(--success-color);
}

.log-item.is-warn .log-level {
  color: var(--warning-color);
}

.log-item.is-error .log-level {
  color: var(--danger-color);
}

.log-item.is-debug .log-level {
  color: var(--text-faint);
}

.log-target {
  font-size: 12px;
  color: var(--primary-color);
  background: var(--primary-soft);
  border-radius: 999px;
  padding: 1px 8px;
  justify-self: start;
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.log-text {
  font-size: 12.5px;
  line-height: 1.6;
  color: var(--text-secondary);
  word-break: break-word;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  cursor: pointer;
}

.log-text.is-open {
  display: block;
  -webkit-line-clamp: unset;
  color: var(--text-main);
}

.log-item.is-error .log-text {
  color: var(--danger-color);
}

.log-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 64px 20px;
  color: var(--text-muted);
}

.empty-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 46px;
  height: 46px;
  border-radius: 50%;
  background: var(--panel-subtle);
  color: var(--text-faint);
  font-size: 22px;
}

.empty-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-main);
}

.empty-hint {
  font-size: 12.5px;
  color: var(--text-muted);
  text-align: center;
}

@media (max-width: 960px) {
  .log-item {
    grid-template-columns: 4px 84px 52px minmax(0, 1fr);
  }

  .log-target {
    display: none;
  }
}
</style>
