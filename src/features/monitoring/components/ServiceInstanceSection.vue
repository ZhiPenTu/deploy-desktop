<template>
  <div class="section-head">
    <h3>服务实例</h3>
    <span>{{ runningCount }} 个运行中，共 {{ containers.length }} 个服务</span>
  </div>

  <div
    class="containers-grid"
    :class="{ 'is-loading': loading }"
    v-loading="loading"
    element-loading-text="正在读取容器状态"
  >
    <div
      v-for="c in containers"
      :key="c.name"
      class="container-card"
      :class="c.state === 'running' ? 'is-running' : 'is-stopped'"
    >
      <div class="c-head">
        <div class="c-ident">
          <span class="c-icon"><el-icon><Platform /></el-icon></span>
          <div class="title-wrap">
            <span class="c-name" :title="c.name">{{ c.name }}</span>
            <span class="c-role">{{ roleDesc(c.service_key) }}</span>
          </div>
        </div>
        <span class="status-pill" :class="statusClass(c)">{{ statusText(c) }}</span>
      </div>

      <div class="c-body">
        <div class="info-row">
          <span class="k">运行状态</span>
          <span class="v">{{ c.status_text || '未运行' }}</span>
        </div>
        <div class="info-row" v-if="c.ports">
          <span class="k">端口映射</span>
          <span class="v code" :title="c.ports">{{ c.ports }}</span>
        </div>
        <div class="info-row">
          <span class="k">镜像版本</span>
          <span class="v code" :title="c.image">{{ c.image }}</span>
        </div>
      </div>

      <div class="c-foot">
        <el-button size="small" text :disabled="operating" @click="$emit('restart', c.service_key)">
          重启服务
        </el-button>
        <el-button size="small" text type="primary" @click="$emit('view-log', c.name)">
          查看日志
        </el-button>
      </div>
    </div>
  </div>

  <div v-if="!containers.length && !loading" class="cluster-empty">
    <span class="cluster-empty-title">未读取到服务实例</span>
    <span class="cluster-empty-hint">
      {{ readError ? '请确认监控主机可连接后重试。' : '请刷新状态重试，若仍无数据，请确认该主机已完成部署。' }}
    </span>
  </div>
</template>

<script setup>
import { Platform } from '@element-plus/icons-vue';

const props = defineProps({
  containers: {
    type: Array,
    required: true
  },
  loading: {
    type: Boolean,
    default: false
  },
  operating: {
    type: Boolean,
    default: false
  },
  runningCount: {
    type: Number,
    default: 0
  },
  readError: {
    type: String,
    default: ''
  },
  roleMap: {
    type: Object,
    default: () => ({})
  }
});

defineEmits(['restart', 'view-log']);

const roleDesc = (key) => props.roleMap[key] || '系统组件';

const statusText = (c) => {
  if (c.health === 'healthy') return 'HEALTHY';
  if (c.health === 'unhealthy') return 'UNHEALTHY';
  return (c.state || 'unknown').toUpperCase();
};

const statusClass = (c) => {
  if (c.health === 'healthy') return 'good';
  if (c.health === 'unhealthy') return 'bad';
  return c.state === 'running' ? 'info' : 'bad';
};
</script>

<style scoped>
.section-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  margin-bottom: 14px;
}

.section-head h3 {
  font-size: 15px;
  font-weight: 650;
  color: var(--text-main);
}

.section-head span {
  font-size: 12px;
  color: var(--text-muted);
}

.containers-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 16px;
}

.containers-grid.is-loading {
  min-height: 120px;
}

.cluster-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 140px;
  padding: 28px;
  text-align: center;
  background: var(--panel-bg);
  border: 1px dashed var(--border-color);
  border-radius: var(--radius-md);
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

.container-card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
  transition: border-color 0.18s ease, box-shadow 0.18s ease, transform 0.18s ease;
}

.container-card:hover {
  border-color: var(--brand-border);
  box-shadow: var(--shadow-card-hover);
  transform: translateY(-1px);
}

.c-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
  padding: 16px 18px 14px;
  border-bottom: 1px solid var(--border-subtle);
}

.c-ident {
  display: flex;
  align-items: center;
  gap: 11px;
  min-width: 0;
}

.c-icon {
  width: 34px;
  height: 34px;
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-sm);
  background: var(--surface-muted);
  color: var(--text-secondary);
  font-size: 16px;
}

.container-card.is-running .c-icon {
  background: var(--success-soft);
  color: var(--success-color);
}

.title-wrap {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.c-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
  font-family: ui-monospace, "SFMono-Regular", Menlo, Monaco, Consolas, monospace;
  line-height: 1.35;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.c-role {
  margin-top: 2px;
  font-size: 11.5px;
  color: var(--text-muted);
}

.status-pill {
  flex: none;
  padding: 2px 9px;
  border-radius: 999px;
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.status-pill.good {
  color: var(--success-color);
  background: var(--success-soft);
}

.status-pill.info {
  color: var(--primary-color);
  background: var(--primary-soft);
}

.status-pill.bad {
  color: var(--danger-color);
  background: var(--danger-soft);
}

.c-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 18px;
  font-size: 12px;
}

.info-row {
  display: flex;
  justify-content: space-between;
  gap: 10px;
}

.info-row .k {
  flex: none;
  color: var(--text-muted);
}

.info-row .v {
  min-width: 0;
  text-align: right;
  color: var(--text-main);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.info-row .v.code {
  font-family: ui-monospace, "SFMono-Regular", Menlo, Monaco, Consolas, monospace;
  color: var(--text-secondary);
}

.c-foot {
  display: flex;
  justify-content: flex-end;
  gap: 4px;
  padding: 8px 14px;
  border-top: 1px solid var(--border-subtle);
  background: var(--panel-subtle);
}

@media (max-width: 1080px) {
  .containers-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
