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
  height: 60px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 0 32px;
  background: rgba(255, 255, 255, 0.92);
  border-bottom: 1px solid var(--border-color);
}

.crumb {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  font-size: 12px;
  color: var(--text-faint);
}

.crumb-sep {
  color: #d0d5dd;
}

.crumb-current {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
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
