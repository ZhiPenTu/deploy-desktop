<template>
  <div class="page-view dashboard-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">集群服务监控</h2>
        <p class="page-subtitle">监控核心容器的健康探针与运行指标</p>
      </div>

      <div class="header-actions">
        <el-button type="success" size="default" :loading="operating" @click="handleStackAction('up')">
          <el-icon><VideoPlay /></el-icon>
          <span>启动全栈</span>
        </el-button>
        <el-button type="warning" size="default" :loading="operating" @click="handleStackAction('restart')">
          <el-icon><RefreshRight /></el-icon>
          <span>重启全栈</span>
        </el-button>
        <el-button type="danger" size="default" :loading="operating" @click="handleStackAction('stop')">
          <el-icon><VideoPause /></el-icon>
          <span>停止全栈</span>
        </el-button>
        <el-button size="default" :loading="loading" @click="fetchStatus">
          <el-icon><Refresh /></el-icon>
          <span>刷新状态</span>
        </el-button>
      </div>
    </div>

    <div class="quick-links">
      <div class="link-item">
        <span class="icon-tile">
          <el-icon><Monitor /></el-icon>
        </span>
        <div class="txt">
          <div class="title">Web 控制台</div>
          <a :href="`http://${targetIp}:18080`" target="_blank" class="url">{{ targetIp }}:18080</a>
        </div>
      </div>
      <div class="link-item">
        <span class="icon-tile neutral">
          <el-icon><Link /></el-icon>
        </span>
        <div class="txt">
          <div class="title">Spring Boot 接口 (主)</div>
          <a :href="`http://${targetIp}:8000`" target="_blank" class="url">{{ targetIp }}:8000</a>
        </div>
      </div>
      <div class="link-item">
        <span class="icon-tile success">
          <el-icon><Connection /></el-icon>
        </span>
        <div class="txt">
          <div class="title">设备通信 UDP 端口</div>
          <span class="url text-neutral">{{ targetIp }}:20000</span>
        </div>
      </div>
    </div>

    <div class="containers-grid">
      <div
        v-for="c in containers"
        :key="c.name"
        class="container-card"
        :class="{ running: c.state === 'running', stopped: c.state !== 'running' }"
      >
        <div class="c-head">
          <div class="title-wrap">
            <span class="name-line">
              <span class="state-dot" :class="{ running: c.state === 'running' }"></span>
              <span class="c-name">{{ c.name }}</span>
            </span>
            <span class="c-role">{{ getRoleDesc(c.service_key) }}</span>
          </div>
          <div class="tags-wrap">
            <el-tag
              :type="c.health === 'healthy' ? 'success' : c.health === 'unhealthy' ? 'danger' : 'info'"
              size="small"
            >
              {{ c.health === 'healthy' ? 'HEALTHY' : c.health === 'unhealthy' ? 'UNHEALTHY' : c.state.toUpperCase() }}
            </el-tag>
          </div>
        </div>

        <div class="c-body">
          <div class="info-row">
            <span class="k">运行状态:</span>
            <span class="v">{{ c.status_text || '未运行' }}</span>
          </div>
          <div class="info-row" v-if="c.ports">
            <span class="k">端口映射:</span>
            <span class="v code" :title="c.ports">{{ c.ports }}</span>
          </div>
          <div class="info-row">
            <span class="k">镜像版本:</span>
            <span class="v code" :title="c.image">{{ c.image }}</span>
          </div>
        </div>

        <div class="c-foot">
          <el-button size="small" plain @click="handleRestartService(c.service_key)">
            重启此服务
          </el-button>
          <el-button size="small" type="primary" plain @click="$emit('view-log', c.name)">
            查看实时日志
          </el-button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue';
import {
  Connection,
  Link,
  Monitor,
  Refresh,
  RefreshRight,
  VideoPause,
  VideoPlay
} from '@element-plus/icons-vue';
import { ElMessage } from 'element-plus';
import {
  getContainersStatus,
  dockerComposeAction,
  getRemoteContainersStatus,
  remoteDockerComposeAction
} from '../api/tauri.js';
import { defaultCluster } from '../config/cluster.js';

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

const roleMap = Object.fromEntries(
  defaultCluster.services.map((service) => [service.serviceKey, service.description])
);

const getRoleDesc = (key) => roleMap[key] || '系统组件';

const fetchStatus = async () => {
  loading.value = true;
  try {
    if (isRemote.value) {
      containers.value = await getRemoteContainersStatus(
        props.connection.remoteHost,
        props.connection.remotePort,
        props.connection.remoteUser,
        props.connection.remoteKey || null,
        props.connection.installDir
      );
    } else {
      containers.value = await getContainersStatus();
    }
  } catch (e) {
    ElMessage.error(`读取容器状态失败: ${e}`);
  } finally {
    loading.value = false;
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
.dashboard-view {
  display: block;
}

.header-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}

.quick-links {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
  margin-bottom: 24px;
}

.link-item {
  background-color: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
  padding: 14px 16px;
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
}

.link-item .icon-tile {
  width: 38px;
  height: 38px;
  border-radius: 10px;
  font-size: 18px;
}

.link-item .txt {
  flex: 1;
  min-width: 0;
}

.link-item .title {
  font-size: 11px;
  color: var(--text-muted);
  margin-bottom: 3px;
}

.link-item .url {
  font-size: 13px;
  font-weight: 600;
  color: #8ab6ff;
  text-decoration: none;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  display: block;
}

.link-item .text-neutral {
  color: #f1f5f9;
}

.containers-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
}

.container-card {
  background-color: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transition: border-color 0.18s ease, transform 0.18s ease, background-color 0.18s ease;
}

.container-card:hover {
  border-color: var(--border-strong);
  background-color: var(--card-bg-hover);
  transform: translateY(-1px);
}

.container-card.running {
  border-top: 2px solid var(--success-color);
}

.container-card.stopped {
  border-top: 2px solid var(--danger-color);
  opacity: 0.9;
}

.c-head {
  padding: 14px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
}

.title-wrap {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
}

.name-line {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}

.state-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex: none;
  background: var(--danger-color);
  box-shadow: 0 0 0 3px rgba(239, 106, 106, 0.12);
}

.state-dot.running {
  background: var(--success-color);
  box-shadow: 0 0 0 3px rgba(53, 196, 143, 0.12);
}

.c-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  line-height: 1.35;
  overflow-wrap: anywhere;
}

.c-role {
  font-size: 11px;
  color: #94a3b8;
  margin-top: 2px;
}

.c-body {
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 12px;
  flex: 1;
}

.info-row {
  display: flex;
  justify-content: space-between;
  gap: 8px;
}

.info-row .k {
  color: var(--text-muted);
  white-space: nowrap;
}

.info-row .v {
  color: #e2e8f0;
  text-align: right;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.info-row .v.code {
  font-family: monospace;
}

.c-foot {
  padding: 12px 14px;
  border-top: 1px solid rgba(255, 255, 255, 0.05);
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  background-color: rgba(0, 0, 0, 0.15);
}
</style>
