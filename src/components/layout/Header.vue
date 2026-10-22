<template>
  <header class="app-header" data-tauri-drag-region="deep">
    <div class="crumb">
      <span class="crumb-root">工作区</span>
      <span class="crumb-sep">/</span>
      <span class="crumb-group">{{ group }}</span>
      <span class="crumb-sep">/</span>
      <span class="crumb-current">{{ pageTitle }}</span>
    </div>

    <div class="header-actions" data-tauri-drag-region="false">
      <UpdateChecker />
    </div>
  </header>
</template>

<script setup>
import UpdateChecker from '../../features/settings/components/UpdateChecker.vue';

defineProps({
  pageTitle: {
    type: String,
    default: '部署向导'
  },
  group: {
    type: String,
    default: '部署'
  }
});
</script>

<style scoped>
.app-header {
  height: 56px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 0 32px;
  background: rgba(255, 255, 255, 0.88);
  backdrop-filter: blur(12px);
  border-bottom: 1px solid var(--border-color);
  box-shadow: 0 1px 2px 0 rgba(15, 23, 42, 0.03);
}

.crumb {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  font-size: 12.5px;
  color: var(--text-faint);
}

.crumb-root {
  color: var(--text-muted);
}

.crumb-sep {
  color: var(--border-strong);
  font-size: 11px;
}

.crumb-group {
  color: var(--text-secondary);
  font-weight: 500;
}

.crumb-current {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  padding: 2px 9px;
  border-radius: var(--radius-xs);
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 9px;
  flex: none;
}

:deep(.el-button) {
  height: 32px;
  padding: 0 12px;
}
</style>
