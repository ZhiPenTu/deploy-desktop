<template>
  <div class="page-view update-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">服务更新</h2>
        <p class="page-subtitle">对已部署的目标机执行增量更新，只替换产物并重启受影响的服务</p>
      </div>
      <div class="header-actions">
        <el-tag v-if="isRemote" type="primary" effect="plain">
          远程 {{ connection.remoteHost }}
        </el-tag>
        <el-tag v-else type="info" effect="plain">本地部署</el-tag>
        <el-button :loading="checking" :disabled="!isRemote" @click="checkDeployment">
          <el-icon><Refresh /></el-icon>
          <span>检测部署状态</span>
        </el-button>
      </div>
    </div>

    <el-alert
      v-if="!isRemote"
      title="服务更新面向局域网远程部署"
      description="本地模式请使用部署向导完成首次安装。远程模式可在目标机已部署的情况下执行增量更新。"
      type="info"
      :closable="false"
      show-icon
      class="mode-alert"
    />

    <el-alert
      v-if="isRemote && deployment && !deployment.deployed"
      title="目标机尚未部署服务"
      description="未检测到可更新的部署目录。空机器请先使用“部署向导”完成首次离线部署，部署完成后再回到本页执行增量更新。"
      type="warning"
      :closable="false"
      show-icon
      class="mode-alert"
    />

    <div v-if="isRemote" class="update-grid">
      <div class="card-surface target-card">
        <div class="card-title-bar">
          <h3>目标机连接</h3>
          <span class="hint">与部署向导共用连接配置</span>
        </div>
        <el-form label-width="92px" label-position="left">
          <el-form-item label="主机 IP">
            <el-input v-model="connection.remoteHost" placeholder="例如: 192.0.2.10" />
          </el-form-item>
          <el-form-item label="SSH 端口">
            <el-input-number v-model="connection.remotePort" :min="1" :max="65535" style="width: 100%" />
          </el-form-item>
          <el-form-item label="登录用户">
            <el-input v-model="connection.remoteUser" placeholder="默认: root" />
          </el-form-item>
          <el-form-item label="私钥路径">
            <el-input v-model="connection.remoteKey" placeholder="留空自动使用 ~/.ssh/id_ed25519_<host>" />
          </el-form-item>
          <el-form-item label="部署根目录">
            <el-input v-model="connection.installDir" placeholder="/opt/deploy" />
          </el-form-item>
        </el-form>
      </div>

      <div v-loading="checking" class="card-surface status-card">
        <div class="card-title-bar">
          <h3>部署状态</h3>
          <span v-if="deployment?.deployed" class="hint">
            {{ deployment.running_containers }} / {{ deployment.total_containers }} 个容器运行中
          </span>
        </div>

        <template v-if="deployment">
          <div class="status-line">
            <el-tag :type="deployment.deployed ? 'success' : 'danger'" size="small">
              {{ deployment.deployed ? '可执行更新' : '尚未部署' }}
            </el-tag>
            <span class="status-text">{{ deployment.message }}</span>
          </div>

          <div class="artifact-list">
            <div class="artifact-row">
              <span class="k">后端 fat jar</span>
              <span class="v">{{ artifactText(deployment.server_jar) }}</span>
            </div>
            <div class="artifact-row">
              <span class="k">网关 classes</span>
              <span class="v">{{ artifactText(deployment.gateway_classes) }}</span>
            </div>
            <div class="artifact-row">
              <span class="k">网关 lib</span>
              <span class="v">{{ artifactText(deployment.gateway_lib) }}</span>
            </div>
            <div class="artifact-row">
              <span class="k">前端产物</span>
              <span class="v">{{ artifactText(deployment.frontend_dist) }}</span>
            </div>
            <div class="artifact-row">
              <span class="k">Compose 文件</span>
              <span class="v">{{ deployment.compose_file || '未找到' }}</span>
            </div>
          </div>

          <div v-if="deployment.issues.length" class="issues">
            <span v-for="issue in deployment.issues" :key="issue" class="issue-chip">{{ issue }}</span>
          </div>
        </template>

        <div v-else class="empty-state">
          点击“检测部署状态”读取目标机的部署目录、产物与容器信息
        </div>
      </div>
    </div>

    <div v-if="isRemote" class="card-surface update-card">
      <div class="card-title-bar">
        <h3>更新内容</h3>
        <span class="hint">更新包目录需包含 app/ 与 deploy/</span>
      </div>

      <el-form label-width="110px">
        <el-form-item label="更新包目录">
          <el-input v-model="bundleDir" placeholder="/tmp/deploy-offline-bundle" />
          <div class="field-hint">
            请填写包含 app/、deploy/、database/ 的离线更新目录。
          </div>
        </el-form-item>

        <el-form-item label="更新组件">
          <el-checkbox-group v-model="selectedComponents" class="component-grid">
            <el-checkbox value="backend" border>
              <span class="component-title">后端服务</span>
              <span class="component-desc">替换 server.jar，重启 backend-java / backend-java-b</span>
            </el-checkbox>
            <el-checkbox value="gateway" border>
              <span class="component-title">网关服务</span>
              <span class="component-desc">替换 classes/lib，重启两个 gateway 分片</span>
            </el-checkbox>
            <el-checkbox value="frontend" border>
              <span class="component-title">前端页面</span>
              <span class="component-desc">替换 dist 并同步到 Nginx 容器</span>
            </el-checkbox>
            <el-checkbox value="config" border>
              <span class="component-title">部署配置</span>
              <span class="component-desc">更新 compose/nginx/oceanbase 并执行 compose up -d</span>
            </el-checkbox>
          </el-checkbox-group>
        </el-form-item>

        <el-form-item label="更新前备份">
          <div class="switch-row">
            <el-switch v-model="backup" />
            <span class="field-hint inline">备份到目标机 backup/update-时间戳，失败时可用于回滚</span>
          </div>
        </el-form-item>

        <el-form-item>
          <div class="action-row">
            <el-button
              size="large"
              :loading="preflighting"
              :disabled="!canPreflight"
              @click="handlePreflight"
            >
              仅预检
            </el-button>
            <el-button
              type="primary"
              size="large"
              :loading="updating"
              :disabled="!canUpdate"
              @click="handleUpdate"
            >
              <el-icon><UploadFilled /></el-icon>
              <span>开始更新所选服务</span>
            </el-button>
          </div>
        </el-form-item>
      </el-form>

      <el-alert
        title="更新会重启所选服务"
        description="OceanBase / Kafka / Redis 数据卷不会被删除；如同时更新部署配置，会执行 docker compose up -d。"
        type="warning"
        :closable="false"
        show-icon
      />
    </div>

    <div v-if="progress.length || result" class="card-surface progress-card">
      <div class="card-title-bar">
        <h3>更新进度</h3>
        <el-tag v-if="result" :type="result.success ? 'success' : 'danger'" size="small">
          {{ result.success ? (lastDryRun ? '预检通过' : '更新成功') : '更新失败' }}
        </el-tag>
      </div>

      <div class="progress-list">
        <div v-for="(item, index) in progress" :key="index" class="progress-row" :class="item.status">
          <span class="progress-dot"></span>
          <div class="progress-copy">
            <span class="progress-step">{{ item.step }}</span>
            <span class="progress-msg">{{ item.message }}</span>
          </div>
          <span class="progress-percent">{{ item.percent }}%</span>
        </div>
      </div>

      <el-progress
        v-if="updating"
        :percentage="currentPercent"
        :stroke-width="8"
        :show-text="false"
        class="progress-bar"
      />

      <div v-if="result" class="result-box">
        <div v-if="result.backup_dir" class="result-line">
          备份目录: <code>{{ result.backup_dir }}</code>
        </div>
        <div class="result-line">耗时: {{ result.duration_secs }} 秒</div>
        <div v-if="result.error" class="result-error">{{ result.error }}</div>
        <pre v-if="result.output" class="output-log">{{ result.output }}</pre>
        <el-button v-if="result.success && !lastDryRun" type="primary" plain @click="$emit('go-dashboard')">
          查看集群监控
        </el-button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Refresh, UploadFilled } from '@element-plus/icons-vue';
import {
  inspectRemoteDeployment,
  onRemoteUpdateProgress,
  updateRemoteDeployment
} from '../api/tauri.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

defineEmits(['go-dashboard']);

const isRemote = computed(() => props.connection.mode === 'remote');
const checking = ref(false);
const updating = ref(false);
const preflighting = ref(false);
const lastDryRun = ref(false);
const deployment = ref(null);
const bundleDir = ref('/tmp/deploy-offline-bundle');
const selectedComponents = ref(['backend', 'gateway', 'frontend']);
const backup = ref(true);
const progress = ref([]);
const result = ref(null);
let unlistenProgress = null;

const componentNames = {
  backend: '后端服务',
  gateway: '网关服务',
  frontend: '前端页面',
  config: '部署配置'
};

const canPreflight = computed(
  () =>
    isRemote.value &&
    deployment.value?.deployed &&
    selectedComponents.value.length > 0 &&
    bundleDir.value.trim().length > 0 &&
    !updating.value &&
    !preflighting.value
);

const canUpdate = computed(() => canPreflight.value && !updating.value);

const currentPercent = computed(() =>
  progress.value.length ? progress.value[progress.value.length - 1].percent : 0
);

const componentLabel = (keys) => keys.map((key) => componentNames[key] || key).join('、');

const artifactText = (artifact) => {
  if (!artifact?.exists) {
    return '未找到';
  }
  if (artifact.size_bytes > 0) {
    const size = (artifact.size_bytes / 1024 / 1024).toFixed(1);
    return `${size} MB · ${artifact.modified || '时间未知'}`;
  }
  return artifact.modified || '已存在';
};

const checkDeployment = async () => {
  if (!isRemote.value) {
    return;
  }
  checking.value = true;
  try {
    deployment.value = await inspectRemoteDeployment(
      props.connection.remoteHost,
      props.connection.remotePort,
      props.connection.remoteUser,
      props.connection.remoteKey || null,
      props.connection.installDir
    );
  } catch (e) {
    deployment.value = null;
    ElMessage.error(`检测部署状态失败: ${e}`);
  } finally {
    checking.value = false;
  }
};

const validateUpdateInputs = () => {
  if (!deployment.value?.deployed) {
    ElMessage.warning('目标机尚未检测到已部署服务，请先完成首次部署');
    return false;
  }
  if (selectedComponents.value.length === 0) {
    ElMessage.warning('请至少选择一个更新组件');
    return false;
  }
  if (!bundleDir.value.trim()) {
    ElMessage.warning('请填写本地更新包目录');
    return false;
  }
  return true;
};

const runUpdate = async (dryRun) => {
  if (!validateUpdateInputs()) {
    return;
  }

  if (!dryRun) {
    try {
      await ElMessageBox.confirm(
        `将更新 ${props.connection.remoteHost} 上的${componentLabel(selectedComponents.value)}，并重启受影响的服务。是否继续？`,
        '确认服务更新',
        {
          confirmButtonText: '开始更新',
          cancelButtonText: '取消',
          type: 'warning'
        }
      );
    } catch {
      return;
    }
  }

  lastDryRun.value = dryRun;
  if (dryRun) {
    preflighting.value = true;
  } else {
    updating.value = true;
  }
  result.value = null;
  progress.value = [];

  try {
    const res = await updateRemoteDeployment(
      props.connection.remoteHost,
      props.connection.remotePort,
      props.connection.remoteUser,
      props.connection.remoteKey || null,
      bundleDir.value.trim(),
      props.connection.installDir,
      selectedComponents.value,
      backup.value,
      dryRun
    );
    result.value = res;
    if (res.status) {
      deployment.value = res.status;
    }
    ElMessage.success(dryRun ? '预检通过' : '服务更新完成');
  } catch (e) {
    result.value = {
      success: false,
      components: selectedComponents.value,
      backup_dir: null,
      output: '',
      duration_secs: 0,
      error: String(e)
    };
    ElMessage.error(dryRun ? '预检失败' : '服务更新失败');
  } finally {
    preflighting.value = false;
    updating.value = false;
  }
};

const handlePreflight = () => runUpdate(true);
const handleUpdate = () => runUpdate(false);

onMounted(async () => {
  if (isRemote.value) {
    await checkDeployment();
  }
  unlistenProgress = await onRemoteUpdateProgress((payload) => {
    progress.value.push(payload);
  });
});

onUnmounted(() => {
  if (typeof unlistenProgress === 'function') {
    unlistenProgress();
  }
});
</script>

<style scoped>
.update-view {
  display: block;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.mode-alert {
  margin-bottom: 20px;
}

.update-grid {
  display: grid;
  grid-template-columns: minmax(320px, 0.9fr) minmax(0, 1.1fr);
  gap: 18px;
  margin-bottom: 18px;
}

.target-card,
.status-card,
.update-card,
.progress-card {
  padding: 20px;
}

.card-title-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 14px;
  margin-bottom: 16px;
}

.card-title-bar h3 {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-main);
}

.card-title-bar .hint {
  font-size: 11px;
  color: var(--text-muted);
}

.status-line {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  margin-bottom: 14px;
}

.status-text {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.artifact-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: rgba(255, 255, 255, 0.018);
}

.artifact-row {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  gap: 14px;
  font-size: 12px;
}

.artifact-row .k {
  color: var(--text-muted);
  flex: none;
}

.artifact-row .v {
  color: var(--text-secondary);
  text-align: right;
  overflow-wrap: anywhere;
}

.issues {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 12px;
}

.issue-chip {
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 999px;
  color: #f2c46d;
  background: rgba(242, 196, 109, 0.1);
  border: 1px solid rgba(242, 196, 109, 0.22);
}

.empty-state {
  min-height: 160px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  font-size: 12px;
  border: 1px dashed var(--border-color);
  border-radius: var(--radius-sm);
}

.update-card {
  margin-bottom: 18px;
}

.field-hint {
  width: 100%;
  margin-top: 6px;
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-muted);
}

.field-hint.inline {
  width: auto;
  margin-top: 0;
}

.switch-row {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.action-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.component-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
  width: 100%;
}

.component-grid :deep(.el-checkbox) {
  height: auto;
  min-height: 64px;
  margin: 0;
  padding: 10px 12px;
  align-items: flex-start;
  white-space: normal;
}

.component-grid :deep(.el-checkbox__label) {
  display: flex;
  flex-direction: column;
  gap: 3px;
  line-height: 1.45;
}

.component-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
}

.component-desc {
  font-size: 11px;
  color: var(--text-muted);
}

.progress-card {
  margin-bottom: 18px;
}

.progress-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-bottom: 14px;
}

.progress-row {
  display: grid;
  grid-template-columns: 12px minmax(0, 1fr) 48px;
  align-items: start;
  gap: 10px;
  padding: 8px 10px;
  border-radius: 8px;
}

.progress-row:nth-child(odd) {
  background: rgba(255, 255, 255, 0.018);
}

.progress-dot {
  width: 7px;
  height: 7px;
  margin-top: 5px;
  border-radius: 50%;
  background: var(--primary-color);
}

.progress-row.success .progress-dot {
  background: var(--success-color);
}

.progress-row.error .progress-dot,
.progress-row.skipped .progress-dot {
  background: #7a8699;
}

.progress-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.progress-step {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-main);
}

.progress-msg {
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-muted);
  overflow-wrap: anywhere;
}

.progress-percent {
  font-size: 11px;
  color: var(--text-muted);
  text-align: right;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
}

.progress-bar {
  margin-bottom: 14px;
}

.result-box {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--border-color);
}

.result-line {
  font-size: 12px;
  color: var(--text-secondary);
}

.result-line code {
  color: #8ab6ff;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
}

.result-error {
  font-size: 12px;
  line-height: 1.6;
  color: #ff8d8d;
  white-space: pre-wrap;
}

.output-log {
  max-height: 240px;
  overflow: auto;
  margin: 0;
  padding: 12px;
  border-radius: var(--radius-sm);
  background: #0b0f17;
  border: 1px solid var(--border-color);
  color: #b9c6d8;
  font-size: 11px;
  line-height: 1.55;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: pre-wrap;
}

@media (max-width: 1080px) {
  .update-grid {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 760px) {
  .component-grid {
    grid-template-columns: 1fr;
  }
}
</style>
