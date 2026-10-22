<template>
  <div v-if="!services.length" class="endpoint-empty card-surface">
    <span class="icon-tile neutral"><el-icon><Connection /></el-icon></span>
    <span class="cluster-empty-title">当前项目还没有可展示的服务端口</span>
    <span class="cluster-empty-hint">选择并绑定交付包后，这里会显示交付包声明的服务地址。</span>
  </div>
  <div v-else class="endpoint-grid">
    <div
      v-for="service in services"
      :key="service.serviceKey"
      class="endpoint-card"
      :class="{ empty: !targetIp }"
    >
      <div class="endpoint-top">
        <span class="endpoint-icon"><el-icon><Connection /></el-icon></span>
      </div>
      <div class="endpoint-label">{{ service.label }}</div>
      <div class="endpoint-value">{{ targetIp ? `${targetIp}:${service.port}` : '未填写目标主机' }}</div>
      <div class="endpoint-desc">{{ service.description || `端口 ${service.port} · ${service.protocol}` }}</div>
    </div>

    <div class="endpoint-card health-card" :class="health.state">
      <div class="endpoint-top">
        <span class="endpoint-icon"><el-icon><CircleCheck /></el-icon></span>
        <span class="health-pill" :class="health.state">{{ health.label }}</span>
      </div>
      <div class="endpoint-label">集群健康</div>
      <div class="endpoint-value">{{ health.healthy }} / {{ health.total }}</div>
      <div class="endpoint-desc">{{ health.description }}</div>
    </div>
  </div>
</template>

<script setup>
import { CircleCheck, Connection } from '@element-plus/icons-vue';

defineProps({
  services: {
    type: Array,
    required: true
  },
  targetIp: {
    type: String,
    default: ''
  },
  health: {
    type: Object,
    required: true
  }
});
</script>

<style scoped>
.endpoint-empty {
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: center;
  justify-content: center;
  min-height: 140px;
  margin-bottom: 28px;
  padding: 24px;
  text-align: center;
}

.endpoint-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 16px;
  margin-bottom: 28px;
}

.endpoint-card {
  display: flex;
  flex-direction: column;
  padding: 18px 20px;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
  transition: border-color 0.18s ease, box-shadow 0.18s ease, transform 0.18s ease;
}

.endpoint-card:hover {
  border-color: var(--brand-border);
  box-shadow: var(--shadow-card-hover);
  transform: translateY(-1px);
}

.endpoint-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}

.endpoint-icon {
  width: 34px;
  height: 34px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-sm);
  background: var(--primary-soft);
  color: var(--primary-color);
  font-size: 16px;
}

.endpoint-label {
  font-size: 11.5px;
  font-weight: 500;
  letter-spacing: 0.02em;
  color: var(--text-muted);
}

.endpoint-value {
  margin-top: 6px;
  font-size: 16px;
  font-weight: 650;
  letter-spacing: -0.01em;
  color: var(--text-main);
  font-family: ui-monospace, "SFMono-Regular", Menlo, Monaco, Consolas, monospace;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.endpoint-desc {
  margin-top: 5px;
  font-size: 11.5px;
  color: var(--text-muted);
}

.endpoint-card.empty .endpoint-value {
  font-size: 13px;
  font-family: inherit;
  font-weight: 500;
  color: var(--text-faint);
}

.endpoint-card.empty .endpoint-icon {
  background: var(--surface-muted);
  color: var(--text-faint);
}

.health-card .endpoint-icon {
  background: var(--success-soft);
  color: var(--success-color);
}

.health-card.warn .endpoint-icon {
  background: var(--warning-soft);
  color: var(--warning-color);
}

.health-card.neutral .endpoint-icon {
  background: var(--surface-muted);
  color: var(--text-secondary);
}

.health-pill {
  display: inline-flex;
  align-items: center;
  padding: 2px 9px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.health-pill.good {
  color: var(--success-color);
  background: var(--success-soft);
}

.health-pill.warn {
  color: var(--warning-color);
  background: var(--warning-soft);
}

.health-pill.neutral {
  color: var(--text-secondary);
  background: var(--surface-muted);
}

.cluster-empty-title {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text-main);
}

.cluster-empty-hint {
  font-size: 12px;
  color: var(--text-muted);
}

@media (max-width: 1080px) {
  .endpoint-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
