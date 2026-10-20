<template>
  <div class="page-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">主机状态</h2>
        <p class="page-subtitle">{{ targetSummary }}</p>
      </div>
      <div class="header-actions">
        <el-button @click="$emit('go-hosts')">返回主机列表</el-button>
        <el-button type="primary" :loading="loading" @click="fetchEnv">
          <el-icon><Refresh /></el-icon>
          <span>重新检查</span>
        </el-button>
      </div>
    </div>

    <el-alert
      v-if="!host"
      title="暂不可用"
      description="请先在主机列表中选择要检查的主机。"
      type="info"
      show-icon
      :closable="false"
    />

    <div class="section-card card-surface host-summary" v-if="host">
      <div class="summary-title">{{ host.name }}</div>
      <div class="summary-tags">
        <el-tag size="small" :type="isRemote ? 'primary' : 'info'" effect="plain">
          {{ isRemote ? 'SSH 主机' : '本机' }}
        </el-tag>
      </div>
      <div class="summary-grid">
        <div class="info-row">
          <span class="k">{{ isRemote ? 'SSH 地址' : '目标位置' }}</span>
          <span class="v">{{ targetText }}</span>
        </div>
        <div class="info-row">
          <span class="k">部署目录</span>
          <span class="v" :class="deployDir ? 'code' : 'muted'">{{ deployDir || '沿用当前配置' }}</span>
        </div>
      </div>
    </div>

    <div class="metrics-grid" v-if="host && env">
      <div class="metric-box card-surface">
        <span class="icon-tile">
          <el-icon><Monitor /></el-icon>
        </span>
        <div class="info">
          <div class="label">操作系统与架构</div>
          <div class="value">{{ env.os_release }}</div>
          <div class="sub">{{ env.os }} / {{ env.arch }}</div>
        </div>
      </div>

      <div class="metric-box card-surface">
        <span class="icon-tile neutral">
          <el-icon><Cpu /></el-icon>
        </span>
        <div class="info">
          <div class="label">CPU 核心数</div>
          <div class="value">{{ env.cpu_cores }} 核心</div>
          <div class="sub">多线程并行计算</div>
        </div>
      </div>

      <div class="metric-box card-surface">
        <span class="icon-tile warning">
          <el-icon><Odometer /></el-icon>
        </span>
        <div class="info">
          <div class="label">物理内存容量</div>
          <div class="value">{{ env.total_memory_mb }} MB</div>
          <div class="sub">可用 {{ env.available_memory_mb }} MB，建议不低于 16 GB</div>
        </div>
      </div>

      <div class="metric-box card-surface">
        <span class="icon-tile success">
          <el-icon><FolderOpened /></el-icon>
        </span>
        <div class="info">
          <div class="label">分区剩余容量</div>
          <div class="value">{{ env.disk_available_gb }} GB</div>
          <div class="sub">建议预留不少于 30 GB</div>
        </div>
      </div>
    </div>

    <div class="section-card card-surface" v-if="env">
      <div class="card-title-bar">
        <h3>Docker 容器引擎</h3>
        <el-tag v-if="env.docker_running" type="success">正常运行中</el-tag>
        <el-tag v-else-if="env.docker_installed" type="warning">已安装未启动</el-tag>
        <el-tag v-else type="danger">未安装</el-tag>
      </div>

      <div class="docker-details">
        <div class="detail-row">
          <span class="k">Docker 版本</span>
          <span class="v">{{ env.docker_version || '未检测到' }}</span>
        </div>
        <div class="detail-row">
          <span class="k">Docker Compose</span>
          <span class="v">{{ env.compose_version || '未检测到插件' }}</span>
        </div>
      </div>

      <div v-if="!env.docker_running" class="install-prompt">
        <el-alert
          title="宿主机未运行 Docker 守护进程"
          description="可使用安装包内的 Docker 静态二进制安装程序，离线注册并启动 systemd 服务。"
          type="warning"
          show-icon
          :closable="false"
        />
        <div class="install-action">
          <el-button type="primary" :loading="installing" @click="handleInstallDocker">
            执行离线安装 Docker 引擎
          </el-button>
        </div>
      </div>
    </div>

    <div class="section-card card-surface" v-if="env">
      <div class="card-title-bar">
        <h3>核心端口状态</h3>
        <span class="hint">排查端口冲突与现有服务绑定</span>
      </div>

      <el-table :data="env.ports" stripe border size="small">
        <el-table-column prop="port" label="端口号" width="100" />
        <el-table-column prop="protocol" label="协议" width="90">
          <template #default="{ row }">
            <el-tag :type="row.protocol === 'UDP' ? 'warning' : 'info'" size="small">
              {{ row.protocol }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="service" label="服务名称与职责" />
        <el-table-column label="可用状态" width="130">
          <template #default="{ row }">
            <el-tag :type="row.available ? 'success' : 'danger'" size="small">
              <span class="tag-with-icon">
                <el-icon><CircleCheck v-if="row.available" /><CircleClose v-else /></el-icon>
                {{ row.available ? '端口空闲' : '已占用' }}
              </span>
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="occupying_process" label="占用来源 / 进程">
          <template #default="{ row }">
            <span v-if="row.occupying_process">{{ row.occupying_process }}</span>
            <span v-else class="text-muted">无</span>
          </template>
        </el-table-column>
      </el-table>
    </div>

    <div class="section-card card-surface" v-if="env && env.issues.length > 0">
      <div class="card-title-bar">
        <h3>诊断建议</h3>
      </div>
      <div class="issues-list">
        <div v-for="(iss, idx) in env.issues" :key="idx" class="issue-item">
          <el-icon class="issue-icon"><WarningFilled /></el-icon>
          <span>{{ iss }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue';
import {
  CircleCheck,
  CircleClose,
  Cpu,
  FolderOpened,
  Monitor,
  Odometer,
  Refresh,
  WarningFilled
} from '@element-plus/icons-vue';
import { ElMessage } from 'element-plus';
import {
  checkLocalEnv,
  installOfflineDockerEngine,
  checkRemoteEnv,
  installRemoteDockerEngine
} from '../api/tauri.js';
import { credentialById } from '../connection.js';

const props = defineProps({
  host: {
    type: Object,
    default: null
  },
  connection: {
    type: Object,
    required: true
  }
});

defineEmits(['go-hosts']);

const isRemote = computed(() => props.host?.kind !== 'local');
const loading = ref(false);
const installing = ref(false);
const env = ref(null);
const credential = computed(() => credentialById(props.host?.credentialId));
const deployDir = computed(() => props.host?.deployDir || props.connection.deployDir);
const targetText = computed(() => (
  isRemote.value
    ? `${props.host?.address || '未配置地址'}:${props.host?.port || 22}`
    : '当前电脑'
));
const targetSummary = computed(() => (
  props.host
    ? `检查「${props.host.name}」的硬件指标、Docker 守护进程及关键通信端口状态`
    : '选择主机后检查其硬件指标、Docker 守护进程及关键通信端口状态'
));

const fetchEnv = async () => {
  if (!props.host) return;
  loading.value = true;
  try {
    env.value = isRemote.value
      ? await checkRemoteEnv(
          props.host.address,
          props.host.port || 22,
          credential.value?.username || props.connection.remoteUser || 'root',
          credential.value?.privateKeyPath || props.connection.remoteKey || null
        )
      : await checkLocalEnv(deployDir.value);
  } catch (e) {
    ElMessage.error(`主机状态检查失败: ${e}`);
  } finally {
    loading.value = false;
  }
};

const handleInstallDocker = async () => {
  installing.value = true;
  try {
    if (isRemote.value) {
      await installRemoteDockerEngine(
        props.host.address,
        props.host.port || 22,
        credential.value?.username || props.connection.remoteUser || 'root',
        credential.value?.privateKeyPath || props.connection.remoteKey || null,
        props.connection.bundleDir
      );
    } else {
      await installOfflineDockerEngine(props.connection.bundleDir);
    }
    await fetchEnv();
  } catch (e) {
    ElMessage.error(`Docker 安装失败: ${e}`);
  } finally {
    installing.value = false;
  }
};

watch(
  () => props.host?.id,
  () => {
    env.value = null;
    fetchEnv();
  },
  { immediate: true }
);
</script>

<style scoped>
.metrics-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 16px;
  margin-bottom: 20px;
}

.metric-box {
  padding: 16px;
  display: flex;
  align-items: flex-start;
  gap: 13px;
  min-width: 0;
}

.info {
  min-width: 0;
}

.metric-box .label {
  font-size: 11px;
  color: var(--text-muted);
}

.metric-box .value {
  margin: 5px 0 3px;
  font-size: 16px;
  font-weight: 600;
  color: var(--text-main);
  line-height: 1.35;
  overflow-wrap: anywhere;
}

.metric-box .sub {
  font-size: 10px;
  line-height: 1.45;
  color: var(--text-muted);
}

.section-card {
  padding: 20px;
  margin-bottom: 20px;
}

.card-title-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  margin-bottom: 16px;
}

.card-title-bar h3 {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-main);
}

.card-title-bar .hint {
  font-size: 12px;
  color: var(--text-muted);
}

.docker-details {
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 13px;
}

.detail-row {
  display: flex;
  align-items: baseline;
  gap: 12px;
}

.detail-row .k {
  width: 130px;
  flex: none;
  color: var(--text-muted);
}

.detail-row .v {
  color: var(--text-secondary);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  word-break: break-all;
}

.install-prompt {
  margin-top: 16px;
}

.install-action {
  margin-top: 12px;
}

.issues-list {
  display: flex;
  flex-direction: column;
  gap: 9px;
}

.issue-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  font-size: 13px;
  line-height: 1.5;
  color: #e8b65d;
}

.issue-icon {
  margin-top: 2px;
  flex: none;
}

.header-actions {
  display: flex;
  flex: none;
  gap: 10px;
}

.host-summary {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.summary-title {
  font-size: 17px;
  font-weight: 600;
  color: var(--text-main);
}

.summary-tags {
  margin-top: -8px;
}

.summary-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 8px 28px;
}

.summary-grid .info-row {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.summary-grid .k {
  width: 72px;
  flex: none;
  font-size: 12px;
  color: var(--text-muted);
}

.summary-grid .v {
  min-width: 0;
  font-size: 13px;
  color: var(--text-secondary);
  overflow-wrap: anywhere;
}
</style>
