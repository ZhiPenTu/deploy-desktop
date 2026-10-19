<template>
  <header class="app-header">
    <div class="brand">
      <div class="brand-mark">
        <el-icon :size="21"><Connection /></el-icon>
      </div>
      <div class="brand-text">
        <h1>离线部署助手</h1>
        <span>镜像清单、离线导入与集群运维</span>
      </div>
    </div>

    <div class="header-actions">
      <div class="mode-chip" :class="isRemote ? 'remote' : 'local'">
        <span class="mode-indicator"></span>
        <div class="mode-copy">
          <span class="mode-name">{{ isRemote ? '远程局域网部署' : '本地离线部署' }}</span>
          <span class="mode-target">{{ isRemote ? remoteTarget : '当前工作站' }}</span>
        </div>
      </div>

      <el-button type="primary" plain @click="$emit('quick-check')">
        <el-icon><Refresh /></el-icon>
        <span>刷新状态</span>
      </el-button>
      <UpdateChecker />
    </div>
  </header>
</template>

<script setup>
import { computed } from 'vue';
import { Connection, Refresh } from '@element-plus/icons-vue';
import UpdateChecker from './UpdateChecker.vue';

const props = defineProps({
  deployMode: {
    type: String,
    default: 'local'
  },
  remoteHost: {
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
  height: 64px;
  flex: none;
  background: rgba(15, 23, 36, 0.9);
  backdrop-filter: blur(14px);
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 24px;
}

.brand {
  display: flex;
  align-items: center;
  gap: 13px;
  min-width: 0;
}

.brand-mark {
  width: 38px;
  height: 38px;
  border-radius: 11px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  background: linear-gradient(145deg, #4f8cff, #2f66d0);
  box-shadow: 0 8px 20px rgba(79, 140, 255, 0.28);
  flex: none;
}

.brand-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.brand-text h1 {
  font-size: 16px;
  font-weight: 600;
  letter-spacing: 0.01em;
  color: var(--text-main);
  white-space: nowrap;
}

.brand-text span {
  margin-top: 2px;
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.mode-chip {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 176px;
  padding: 6px 12px;
  border-radius: 10px;
  border: 1px solid var(--border-color);
  background: rgba(255, 255, 255, 0.025);
}

.mode-indicator {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--success-color);
  box-shadow: 0 0 0 4px rgba(53, 196, 143, 0.12);
  flex: none;
}

.mode-chip.remote .mode-indicator {
  background: var(--primary-color);
  box-shadow: 0 0 0 4px rgba(79, 140, 255, 0.13);
}

.mode-copy {
  display: flex;
  flex-direction: column;
  line-height: 1.25;
}

.mode-name {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-main);
}

.mode-target {
  font-size: 10px;
  color: var(--text-muted);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
}
</style>
