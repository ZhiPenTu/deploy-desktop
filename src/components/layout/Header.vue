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
      <span v-if="projectName" class="project-chip" :title="projectName">{{ projectName }}</span>

      <div class="mode-chip" :class="isRemote ? 'remote' : 'local'">
        <span class="mode-indicator"></span>
        <div class="mode-copy">
          <span class="mode-name">{{ isRemote ? '远程局域网部署' : '本地离线部署' }}</span>
          <span class="mode-target">{{ isRemote ? remoteTarget : '当前工作站' }}</span>
        </div>
      </div>

      <span class="action-divider"></span>

      <el-button plain @click="$emit('quick-check')">
        <el-icon><Refresh /></el-icon>
        <span>刷新状态</span>
      </el-button>
      <UpdateChecker />
    </div>
  </header>
</template>

<script setup>
import { computed } from 'vue';
import { Refresh } from '@element-plus/icons-vue';
import UpdateChecker from '../../features/settings/components/UpdateChecker.vue';

const props = defineProps({
  pageTitle: {
    type: String,
    default: '部署向导'
  },
  group: {
    type: String,
    default: '部署'
  },
  deployMode: {
    type: String,
    default: 'local'
  },
  remoteHost: {
    type: String,
    default: ''
  },
  projectName: {
    type: String,
    default: ''
  }
});

defineEmits(['quick-check']);

const isRemote = computed(() => props.deployMode === 'remote');
const remoteTarget = computed(() => props.remoteHost || '未填写目标主机');
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

.project-chip {
  max-width: 220px;
  padding: 5px 11px;
  border-radius: 9px;
  border: 1px solid var(--border-color);
  background: #f7f9fc;
  color: var(--text-secondary);
  font-size: 11.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mode-chip {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 5px 12px;
  border-radius: 9px;
  border: 1px solid var(--border-color);
  background: #fbfcfe;
}

.mode-indicator {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--success-color);
  box-shadow: 0 0 0 3px rgba(14, 159, 110, 0.12);
  flex: none;
}

.mode-chip.remote .mode-indicator {
  background: var(--primary-color);
  box-shadow: 0 0 0 3px var(--primary-ring);
}

.mode-copy {
  display: flex;
  align-items: baseline;
  gap: 7px;
  line-height: 1.2;
}

.mode-name {
  font-size: 11.5px;
  font-weight: 500;
  color: var(--text-secondary);
}

.mode-target {
  font-size: 11px;
  color: var(--text-muted);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
}

.action-divider {
  width: 1px;
  height: 20px;
  background: var(--border-color);
  margin: 0 2px;
}

:deep(.el-button) {
  height: 32px;
  padding: 0 12px;
}
</style>
