<template>
  <div class="page-view dashboard-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">集群服务监控</h2>
        <p class="page-subtitle">监控核心容器的健康探针与运行指标</p>
      </div>

      <div class="header-actions">
        <el-button type="primary" :loading="operating" @click="handleStackAction('up')">
          <el-icon><VideoPlay /></el-icon>
          <span>启动全栈</span>
        </el-button>
        <el-button :loading="operating" @click="handleStackAction('restart')">
          <el-icon><RefreshRight /></el-icon>
          <span>重启全栈</span>
        </el-button>
        <el-button type="danger" plain :loading="operating" @click="handleStackAction('stop')">
          <el-icon><VideoPause /></el-icon>
          <span>停止全栈</span>
        </el-button>
        <el-button :loading="loading" @click="fetchStatus">
          <el-icon><Refresh /></el-icon>
          <span>刷新状态</span>
        </el-button>
      </div>
    </div>

    <el-alert v-if="readError" :title="readError" type="error" :closable="false" show-icon class="read-error" />

    <div class="endpoint-grid">
      <a
        class="endpoint-card"
        :class="{ empty: !targetIp }"
        :href="targetIp ? `http://${targetIp}:18080` : undefined"
        target="_blank"
      >
        <div class="endpoint-top">
          <span class="endpoint-icon"><el-icon><Monitor /></el-icon></span>
          <el-icon v-if="targetIp" class="endpoint-arrow"><TopRight /></el-icon>
        </div>
        <div class="endpoint-label">Web 控制台</div>
        <div class="endpoint-value">{{ targetIp ? `${targetIp}:18080` : '未填写目标主机' }}</div>
        <div class="endpoint-desc">端口 18080 · 浏览器访问管理界面</div>
      </a>

      <a
        class="endpoint-card"
        :class="{ empty: !targetIp }"
        :href="targetIp ? `http://${targetIp}:8000` : undefined"
        target="_blank"
      >
        <div class="endpoint-top">
          <span class="endpoint-icon"><el-icon><Link /></el-icon></span>
          <el-icon v-if="targetIp" class="endpoint-arrow"><TopRight /></el-icon>
        </div>
        <div class="endpoint-label">Spring Boot 接口</div>
        <div class="endpoint-value">{{ targetIp ? `${targetIp}:8000` : '未填写目标主机' }}</div>
        <div class="endpoint-desc">端口 8000 · 主服务 HTTP 接口</div>
      </a>

      <div class="endpoint-card" :class="{ empty: !targetIp }">
        <div class="endpoint-top">
          <span class="endpoint-icon"><el-icon><Connection /></el-icon></span>
        </div>
        <div class="endpoint-label">设备通信 UDP</div>
        <div class="endpoint-value">{{ targetIp ? `${targetIp}:20000` : '未填写目标主机' }}</div>
        <div class="endpoint-desc">端口 20000 · 网关接收设备上报</div>
      </div>

      <div class="endpoint-card health-card" :class="healthClass">
        <div class="endpoint-top">
          <span class="endpoint-icon"><el-icon><CircleCheck /></el-icon></span>
          <span class="health-pill" :class="healthClass">{{ healthLabel }}</span>
        </div>
        <div class="endpoint-label">集群健康</div>
        <div class="endpoint-value">{{ healthyCount }} / {{ containers.length }}</div>
        <div class="endpoint-desc">{{ healthDesc }}</div>
      </div>
    </div>

    <div class="section-head">
      <h3>服务实例</h3>
      <span>{{ runningCount }} 个运行中，共 {{ containers.length }} 个服务</span>
    </div>

    <div class="containers-grid">
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
              <span class="c-role">{{ getRoleDesc(c.service_key) }}</span>
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
          <el-button size="small" text @click="handleRestartService(c.service_key)">
            重启服务
          </el-button>
          <el-button size="small" text type="primary" @click="$emit('view-log', c.name)">
            查看日志
          </el-button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onScopeDispose } from 'vue';
import { createRequestScope } from '../../../shared/async/requestScope.js';
import {
  CircleCheck,
  Connection,
  Link,
  Monitor,
  Platform,
  Refresh,
  RefreshRight,
  TopRight,
  VideoPause,
  VideoPlay
} from '@element-plus/icons-vue';
import { ElMessage } from 'element-plus';
import {
  getContainersStatus,
  dockerComposeAction,
  getRemoteContainersStatus,
  remoteDockerComposeAction
} from '../../../api/tauri.js';
import { defaultCluster } from '../../../config/cluster.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

defineEmits(['view-log']);

const isRemote = computed(() => props.connection.mode === 'remote');
const targetIp = computed(() => (isRemote.value ? props.connection.remoteHost : '127.0.0.1'));
const deployDir = computed(() => props.connection.deployDir || '/opt/deploy');

const loading = ref(false);
const operating = ref(false);
const containers = ref([]);
const readError = ref('');
const statusRequests = createRequestScope();
onScopeDispose(() => statusRequests.dispose());

const roleMap = Object.fromEntries(
  defaultCluster.services.map((service) => [service.serviceKey, service.description])
);

const getRoleDesc = (key) => roleMap[key] || '系统组件';

const runningCount = computed(() => containers.value.filter((c) => c.state === 'running').length);
const healthyCount = computed(
  () => containers.value.filter((c) => c.state === 'running' && c.health !== 'unhealthy').length
);
const healthClass = computed(() => {
  if (!containers.value.length) return 'neutral';
  if (healthyCount.value === containers.value.length) return 'good';
  return 'warn';
});
const healthLabel = computed(() => {
  if (!containers.value.length) return '未知';
  if (healthyCount.value === containers.value.length) return '正常';
  return '需关注';
});
const healthDesc = computed(() => {
  if (!containers.value.length) return '等待读取容器状态';
  if (healthyCount.value === containers.value.length) return '全部服务健康检查通过';
  return '存在未运行或健康检查异常的服务';
});

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

const fetchStatus = async () => {
  const ticket = statusRequests.begin();
  loading.value = true;
  readError.value = '';
  try {
    let response;
    if (isRemote.value) {
      response = await getRemoteContainersStatus(
        props.connection.remoteHost,
        props.connection.remotePort,
        props.connection.remoteUser,
        props.connection.remoteKey || null,
        props.connection.installDir
      );
    } else {
      response = await getContainersStatus();
    }
    if (statusRequests.isCurrent(ticket)) containers.value = response;
  } catch (e) {
    if (!statusRequests.isCurrent(ticket)) return;
    containers.value = [];
    readError.value = isRemote.value
      ? '无法读取目标主机状态，请确认已连接对应局域网或 VPN 后重试。'
      : '无法读取容器状态，请检查本机 Docker 是否正常运行后重试。';
  } finally {
    if (statusRequests.isCurrent(ticket)) loading.value = false;
  }
};

const handleStackAction = async (action) => {
  operating.value = true;
  try {
    if (isRemote.value) {
      const result = await remoteDockerComposeAction(
        props.connection.remoteHost,
        props.connection.remotePort,
        props.connection.remoteUser,
        props.connection.remoteKey || null,
        deployDir.value,
        action
      );
      if (!result.success) {
        throw new Error(result.error || result.output || '远程操作失败');
      }
    } else {
      const result = await dockerComposeAction(deployDir.value, action);
      if (!result.success) {
        throw new Error(result.error || result.output || '操作失败');
      }
    }
    await fetchStatus();
  } catch (e) {
    ElMessage.error(String(e));
  } finally {
    operating.value = false;
  }
};

const handleRestartService = async (serviceKey) => {
  try {
    if (isRemote.value) {
      const result = await remoteDockerComposeAction(
        props.connection.remoteHost,
        props.connection.remotePort,
        props.connection.remoteUser,
        props.connection.remoteKey || null,
        deployDir.value,
        'restart',
        serviceKey
      );
      if (!result.success) {
        throw new Error(result.error || result.output || '远程重启失败');
      }
    } else {
      const result = await dockerComposeAction(deployDir.value, 'restart', serviceKey);
      if (!result.success) {
        throw new Error(result.error || result.output || '重启失败');
      }
    }
    await fetchStatus();
  } catch (e) {
    ElMessage.error(String(e));
  }
};

onMounted(fetchStatus);
</script>

<style scoped>
.read-error { margin-bottom: 20px; }

.dashboard-view {
  display: block;
}

.header-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.endpoint-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 14px;
  margin-bottom: 28px;
}

.endpoint-card {
  position: relative;
  display: block;
  min-width: 0;
  padding: 16px 18px 17px;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
  text-decoration: none;
  transition: border-color 0.18s ease, box-shadow 0.18s ease, transform 0.18s ease;
}

a.endpoint-card:hover {
  border-color: #c9d2e3;
  box-shadow: var(--shadow-raised);
  transform: translateY(-2px);
}

.endpoint-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 15px;
}

.endpoint-icon {
  width: 30px;
  height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  background: var(--primary-soft);
  color: var(--primary-color);
  font-size: 15px;
}

.endpoint-arrow {
  color: var(--text-faint);
  font-size: 14px;
  transition: color 0.16s ease, transform 0.16s ease;
}

a.endpoint-card:hover .endpoint-arrow {
  color: var(--primary-color);
  transform: translate(1px, -1px);
}

.endpoint-label {
  font-size: 11px;
  font-weight: 500;
  letter-spacing: 0.04em;
  color: var(--text-muted);
}

.endpoint-value {
  margin-top: 5px;
  font-size: 16px;
  font-weight: 600;
  letter-spacing: -0.01em;
  color: var(--text-main);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.endpoint-desc {
  margin-top: 4px;
  font-size: 11px;
  color: var(--text-faint);
}

.endpoint-card.empty .endpoint-value {
  font-size: 13px;
  font-family: inherit;
  font-weight: 500;
  color: var(--text-faint);
}

.endpoint-card.empty .endpoint-icon {
  background: #f1f3f7;
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
  background: #f1f3f7;
  color: var(--text-secondary);
}

.health-pill {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 10.5px;
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
  background: #f1f3f7;
}

.section-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  margin-bottom: 12px;
}

.section-head h3 {
  font-size: 15px;
  font-weight: 600;
}

.section-head span {
  font-size: 12px;
  color: var(--text-muted);
}

.containers-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 14px;
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
  border-color: #c9d2e3;
  box-shadow: var(--shadow-raised);
  transform: translateY(-2px);
}

.c-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
  padding: 16px 16px 14px;
  border-bottom: 1px solid var(--border-color);
}

.c-ident {
  display: flex;
  align-items: center;
  gap: 11px;
  min-width: 0;
}

.c-icon {
  width: 32px;
  height: 32px;
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  background: #f1f3f7;
  color: var(--text-secondary);
  font-size: 15px;
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
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  line-height: 1.35;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.c-role {
  margin-top: 2px;
  font-size: 11px;
  color: var(--text-muted);
}

.status-pill {
  flex: none;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.04em;
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
  padding: 14px 16px;
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
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  color: var(--text-secondary);
}

.c-foot {
  display: flex;
  justify-content: flex-end;
  gap: 4px;
  padding: 8px 12px;
  border-top: 1px solid var(--border-color);
  background: var(--panel-subtle);
}

@media (max-width: 1080px) {
  .endpoint-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .containers-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
