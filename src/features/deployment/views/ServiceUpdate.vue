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
        <el-button :loading="checking" :disabled="!isRemote || updating || preflighting" @click="checkDeployment">
          <el-icon><Refresh /></el-icon>
          <span>检测部署状态</span>
        </el-button>
      </div>
    </div>

    <el-alert
      v-if="!isRemote"
      title="服务更新面向局域网远程部署"
      description="本地部署与增量更新请使用部署向导。远程模式可在目标机已部署的情况下执行增量更新。"
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
        <el-form label-width="92px" label-position="left" :disabled="updating || preflighting">
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
        <span class="hint">更新包目录需包含 images/ + deploy/（镜像模式）或 app/ + deploy/（文件模式）</span>
      </div>

      <el-form label-width="110px" :disabled="updating || preflighting">
        <el-form-item label="更新包目录">
          <el-input v-model="bundleDir" placeholder="/tmp/deploy-offline-bundle" />
          <div class="field-hint">
            请填写制品中心下载的交付包目录。文件模式交付包含 app/ 与 deploy/，替换产物即可升级；
            镜像模式交付包含 images/ 与 deploy/，需要同时导入镜像。
          </div>
          <div v-if="planLoading" class="field-hint">正在读取交付包内容</div>
          <div v-else-if="planError" class="field-hint text-danger">读取交付包失败：{{ planError }}</div>
          <div v-else-if="bundleMode !== 'unknown'" class="field-hint">
            已识别为{{ bundleMode === 'image' ? '镜像模式' : '文件模式' }}交付包
            <template v-if="bundleVersion">，版本 {{ bundleVersion }}</template>
            <template v-if="bundleMode === 'image' && bundleImages.length">
              ，包含 {{ bundleImages.length }} 个镜像包
            </template>
            <template v-else-if="bundlePlan">
              ，含 app/ {{ bundlePlan.appFiles.length }} 个文件、deploy/ {{ bundlePlan.configFiles.length }} 个文件
            </template>
          </div>
        </el-form-item>

        <el-form-item label="更新组件">
          <p v-if="bundleMode === 'file'" class="field-hint block-hint">
            文件模式交付包不含应用镜像：容器直接挂载替换后的 jar、网关 classes 与前端产物，
            基础镜像保持不变，因此本次不需要导入镜像。
          </p>
          <el-checkbox-group v-model="selectedComponents" class="component-grid">
            <el-checkbox v-if="bundleMode !== 'file'" value="image" border>
              <span class="component-title">镜像</span>
              <span class="component-desc">上传镜像包并 docker load，随后按新镜像重建服务</span>
            </el-checkbox>
            <el-checkbox value="backend" border>
              <span class="component-title">后端服务</span>
              <span class="component-desc">替换后端程序并重启相关服务</span>
            </el-checkbox>
            <el-checkbox value="gateway" border>
              <span class="component-title">网关服务</span>
              <span class="component-desc">替换网关程序并重启相关服务</span>
            </el-checkbox>
            <el-checkbox value="frontend" border>
              <span class="component-title">前端页面</span>
              <span class="component-desc">替换 dist 并同步到 Nginx 容器</span>
            </el-checkbox>
            <el-checkbox value="config" border>
              <span class="component-title">部署配置</span>
              <span class="component-desc">更新服务配置并重新启动容器栈</span>
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
        description="已有数据卷会保留；如同时更新部署配置，会重新启动容器栈。"
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
import { useDesktopProgress } from '../../../shared/composables/useDesktopProgress.js';
import { computed, onMounted, onScopeDispose, ref, watch } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Refresh, UploadFilled } from '@element-plus/icons-vue';
import {
  inspectBundlePlan,
  inspectRemoteDeployment,
  onRemoteUpdateProgress
} from '../../../api/tauri.js';
import { createRequestScope } from '../../../shared/async/requestScope.js';
import { createDeploymentContext } from '../domain/context.js';
import { componentLabels, deploymentComponents } from '../domain/components.js';
import { invokeIncremental } from '../services/operations.js';
import { failureText, safeText } from '../domain/errors.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const emit = defineEmits(['go-dashboard', 'busy-change']);

const isRemote = computed(() => props.connection.mode === 'remote');
const checking = ref(false);
const updating = ref(false);
const preflighting = ref(false);
const lastDryRun = ref(false);
const deployment = ref(null);
const bundleDir = ref(props.connection.bundleDir || '');
const selectedComponents = ref(['backend', 'gateway', 'frontend']);
const bundleMode = ref('unknown');
const bundleVersion = ref('');
const bundleImages = ref([]);
const bundlePlan = ref(null);
const planLoading = ref(false);
const planError = ref('');

const planRequests = createRequestScope();
const statusRequests = createRequestScope();
onScopeDispose(() => {
  planRequests.dispose();
  statusRequests.dispose();
  emit('busy-change', false);
});
watch(() => updating.value || preflighting.value, (busy) => emit('busy-change', busy), { flush: 'sync' });

async function loadBundlePlan(value) {
  const ticket = planRequests.begin();
  const dir = String(value || '').trim();
  bundleVersion.value = '';
  bundleImages.value = [];
  selectedComponents.value = [];
  if (!dir) {
    bundleMode.value = 'unknown';
    bundlePlan.value = null;
    planError.value = '';
    planLoading.value = false;
    return;
  }
  planLoading.value = true;
  planError.value = '';
  try {
    const remote = isRemote.value && props.connection.remoteHost
      ? {
          host: props.connection.remoteHost,
          port: props.connection.remotePort,
          user: props.connection.remoteUser,
          keyPath: props.connection.remoteKey || null
        }
      : null;
    const plan = await inspectBundlePlan(dir, remote);
    if (!planRequests.isCurrent(ticket)) return;
    bundlePlan.value = plan;
    bundleMode.value = plan.mode || 'unknown';
    bundleVersion.value = plan.version || '';
    bundleImages.value = (plan.images || []).map((image) => image.imageRef);
    applyComponentDefaults(plan);
  } catch (error) {
    if (!planRequests.isCurrent(ticket)) return;
    bundlePlan.value = null;
    bundleMode.value = 'unknown';
    planError.value = safeText(error);
  } finally {
    if (planRequests.isCurrent(ticket)) planLoading.value = false;
  }
}

function applyComponentDefaults(plan) {
  selectedComponents.value = deploymentComponents(plan);
}

watch(bundleDir, loadBundlePlan, { immediate: true });
const backup = ref(true);
const progress = ref([]);
const result = ref(null);

watch(
  () => props.connection.bundleDir,
  (value) => {
    if (value) {
      bundleDir.value = value;
    }
  }
);

const canPreflight = computed(
  () =>
    isRemote.value &&
    deployment.value?.deployed &&
    selectedComponents.value.length > 0 &&
    bundleDir.value.trim().length > 0 &&
    !planLoading.value &&
    !planError.value &&
    !updating.value &&
    !preflighting.value
);

const canUpdate = computed(() => canPreflight.value && !updating.value);

const currentPercent = computed(() =>
  progress.value.length ? progress.value[progress.value.length - 1].percent : 0
);

const componentLabel = (keys) => keys.map((key) => componentLabels[key] || key).join('、');

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
  const ticket = statusRequests.begin();
  if (!isRemote.value) {
    return;
  }
  checking.value = true;
  try {
    const response = await inspectRemoteDeployment(
      props.connection.remoteHost,
      props.connection.remotePort,
      props.connection.remoteUser,
      props.connection.remoteKey || null,
      props.connection.installDir
    );
    if (statusRequests.isCurrent(ticket)) deployment.value = response;
  } catch (e) {
    if (!statusRequests.isCurrent(ticket)) return;
    deployment.value = null;
    ElMessage.error(failureText(e, '检测部署状态'));
  } finally {
    if (statusRequests.isCurrent(ticket)) checking.value = false;
  }
};

watch(
  () => [props.connection.mode, props.connection.remoteHost, props.connection.remotePort,
    props.connection.remoteUser, props.connection.remoteKey, props.connection.installDir],
  () => {
    statusRequests.invalidate();
    deployment.value = null;
    checking.value = false;
    result.value = null;
    loadBundlePlan(bundleDir.value);
  }
);

const validateUpdateInputs = () => {
  if (!isRemote.value || planLoading.value || planError.value) {
    ElMessage.warning('请先确认目标主机与交付包。');
    return false;
  }
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
  if (updating.value || preflighting.value || !validateUpdateInputs()) {
    return;
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
    const context = createDeploymentContext({
      connection: props.connection, remote: true,
      bundle: { dir: bundleDir.value, version: bundleVersion.value },
      components: selectedComponents.value, backup: backup.value,
      action: 'incremental', label: props.connection.remoteHost
    });
    if (!dryRun) {
      try {
        await ElMessageBox.confirm(
          `目标：${context.host}；目录：${context.installDir}；版本：${context.version || '未标记'}。将更新${componentLabel(context.components)}并重启受影响的服务，${context.backup ? '更新前保留备份' : '本次不备份'}。是否继续？`,
          '确认服务更新',
          { confirmButtonText: '开始更新', cancelButtonText: '取消', type: 'warning', closeOnClickModal: false }
        );
      } catch { return; }
    }
    const res = await invokeIncremental(context, dryRun);
    result.value = { ...res, output: safeText(res.output) };
    if (res.status) {
      deployment.value = res.status;
    }
    ElMessage.success(dryRun ? '预检通过' : '服务更新完成');
  } catch (e) {
    result.value = {
      success: false,
      components: selectedComponents.value,
      backup_dir: null,
      duration_secs: 0,
      ...e.result,
      output: safeText(e.result?.output || ''),
      error: safeText(e)
    };
    ElMessage.error(dryRun ? '预检失败' : '服务更新失败');
  } finally {
    preflighting.value = false;
    updating.value = false;
  }
};

const handlePreflight = () => runUpdate(true);
const handleUpdate = () => runUpdate(false);

useDesktopProgress(onRemoteUpdateProgress, (payload) => {
  progress.value.push(payload);
}, () => ElMessage.warning('进度暂不可用，请等待执行结果。'));

onMounted(async () => {
  if (isRemote.value) await checkDeployment();
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
  background: var(--panel-subtle);
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
  color: var(--warning-color);
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

.field-hint.block-hint {
  margin-top: 0;
  margin-bottom: 8px;
  padding: 8px 12px;
  border-radius: var(--radius-sm);
  background: var(--primary-soft);
  color: var(--text-secondary);
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
  background: var(--panel-subtle);
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
  background: var(--text-muted);
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
  color: var(--primary-color);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
}

.result-error {
  font-size: 12px;
  line-height: 1.6;
  color: var(--danger-color);
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
