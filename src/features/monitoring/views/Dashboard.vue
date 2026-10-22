<template>
  <div class="page-view dashboard-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">集群服务监控</h2>
        <p class="page-subtitle">监控核心容器的健康探针与运行指标</p>
      </div>

      <div class="header-actions">
        <el-button @click="$emit('go-projects')">
          <el-icon><ArrowLeft /></el-icon>
          <span>返回项目</span>
        </el-button>
        <div class="host-switch">
          <span class="host-switch-label">监控目标</span>
          <span class="host-name">{{ targetLabel }}</span>
        </div>
        <span class="action-divider" aria-hidden="true"></span>
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

    <EndpointOverview :services="serviceCards" :target-ip="targetIp" :health="health" />

    <ServiceInstanceSection
      :containers="containers"
      :loading="loading"
      :operating="operating"
      :running-count="runningCount"
      :read-error="readError"
      :role-map="roleMap"
      @restart="handleRestartService"
      @view-log="$emit('view-log', $event)"
    />
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onScopeDispose } from 'vue';
import { createRequestScope } from '../../../shared/async/requestScope.js';
import {
  ArrowLeft,
  Refresh,
  RefreshRight,
  VideoPause,
  VideoPlay
} from '@element-plus/icons-vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import {
  getContainersStatus,
  dockerComposeAction,
  getRemoteContainersStatus,
  remoteDockerComposeAction
} from '../../../api/tauri.js';
import { clusterServices } from '../../../config/cluster.js';
import { projectState } from '../../../config/project.js';
import { hostById, isLocalHost } from '../../connections/state.js';
import { errorText } from '../../../shared/errors.js';
import EndpointOverview from '../components/EndpointOverview.vue';
import ServiceInstanceSection from '../components/ServiceInstanceSection.vue';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const emit = defineEmits(['view-log', 'go-projects']);

const isRemote = computed(() => props.connection.mode === 'remote');
const targetIp = computed(() => (isRemote.value ? props.connection.remoteHost : '127.0.0.1'));
const deployDir = computed(() => props.connection.deployDir || props.connection.installDir);
const services = computed(() => clusterServices());
const serviceCards = computed(() => services.value.filter((service) => service.port).slice(0, 3));
const targetLabel = computed(() => {
  const host = hostById(projectState.project?.hostId || '');
  if (!host) return '尚未配置主机';
  return isLocalHost(host) ? '本机' : `${host.name}（${host.address}）`;
});

const loading = ref(false);
const operating = ref(false);

const containers = ref([]);
const readError = ref('');
const statusRequests = createRequestScope();
onScopeDispose(() => statusRequests.dispose());

const roleMap = computed(() =>
  Object.fromEntries(services.value.map((service) => [service.serviceKey, service.description]))
);

const runningCount = computed(() => containers.value.filter((c) => c.state === 'running').length);
const healthyCount = computed(
  () => containers.value.filter((c) => c.state === 'running' && c.health !== 'unhealthy').length
);
const health = computed(() => {
  const total = containers.value.length;
  if (!total) {
    return { state: 'neutral', label: '未知', healthy: 0, total: 0, description: '等待读取容器状态' };
  }
  const healthy = healthyCount.value;
  if (healthy === total) {
    return { state: 'good', label: '正常', healthy, total, description: '全部服务健康检查通过' };
  }
  return { state: 'warn', label: '需关注', healthy, total, description: '存在未运行或健康检查异常的服务' };
});

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

const stackActionConfirmText = (action) => {
  const projectName = projectState.project?.name || '当前项目';
  const target = isRemote.value ? `${targetLabel.value}` : '本机';
  if (action === 'stop') {
    return `将停止「${projectName}」在${target}的全部服务，停止期间业务不可用。\n\n部署目录：${deployDir.value || '未配置'}\n\n停止后可在此页面重新启动。`;
  }
  if (action === 'restart') {
    return `将重启「${projectName}」在${target}的全部服务，重启期间业务会短暂中断。\n\n部署目录：${deployDir.value || '未配置'}`;
  }
  return '';
};

const handleStackAction = async (action) => {
  const confirmText = stackActionConfirmText(action);
  if (confirmText) {
    try {
      await ElMessageBox.confirm(confirmText, action === 'stop' ? '停止全栈' : '重启全栈', {
        type: 'warning',
        confirmButtonText: action === 'stop' ? '停止' : '重启',
        cancelButtonText: '取消'
      });
    } catch {
      return;
    }
  }
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
    ElMessage.error(errorText(e));
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
    ElMessage.error(errorText(e));
  }
};

onMounted(fetchStatus);
</script>

<style scoped>
.dashboard-view {
  display: block;
}

.read-error {
  margin-bottom: 20px;
}

.header-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.host-switch {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 12px;
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
}

.host-switch-label {
  flex: none;
  font-size: 11.5px;
  color: var(--text-muted);
}

.host-name {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-main);
}

.action-divider {
  width: 1px;
  height: 20px;
  background: var(--border-color);
  margin: 0 4px;
}
</style>
