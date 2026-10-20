<template>
  <div class="page-view wizard-container">
    <div class="page-header">
      <div>
        <h2 class="page-title">部署向导</h2>
        <p class="page-subtitle">选择目标主机与制品，核对服务和镜像后执行首次部署或增量更新</p>
      </div>
    </div>
    <div class="steps-header card-surface">
      <ol class="stepper" aria-label="部署步骤">
        <li
          v-for="(step, index) in steps"
          :key="step"
          class="step"
          :class="{ active: activeStep === index, done: activeStep > index }"
          :aria-current="activeStep === index ? 'step' : undefined"
        >
          <span class="step-node">
            <el-icon v-if="activeStep > index"><Check /></el-icon>
            <span v-else>{{ index + 1 }}</span>
          </span>
          <span>{{ step }}</span>
        </li>
      </ol>
    </div>

    <div v-if="activeStep === 0" class="step-card">
      <h3>选择目标主机</h3>
      <p class="desc">从已保存的主机中选择部署目标，连接信息将自动带入。</p>
      <el-alert
        v-if="connectionInventory.error"
        title="无法读取主机列表，请在主机管理中检查后重试。"
        type="error"
        :closable="false"
        show-icon
      />
      <el-form label-width="110px" class="host-form" @submit.prevent>
        <el-form-item label="目标主机">
          <div class="input-row">
            <el-select
              :model-value="config.activeHostId || undefined"
              :loading="selectingHost"
              :disabled="busy"
              placeholder="请选择目标主机"
              aria-label="目标主机"
              @change="selectHost"
            >
              <el-option
                v-for="host in connectionInventory.hosts"
                :key="host.id"
                :value="host.id"
                :label="
                  host.kind === 'local'
                    ? host.name === '本机'
                      ? '本机'
                      : `${host.name}（本机）`
                    : `${host.name}（${host.address}）`
                "
              />
            </el-select>
            <el-button :disabled="busy" @click="emit('go-hosts')"> 管理主机 </el-button>
          </div>
        </el-form-item>
        <template v-if="selectedHost">
          <el-form-item label="连接信息">
            <span>
              {{ isRemote ? `${config.remoteUser}@${config.remoteHost}:${config.remotePort}` : '本机' }}
            </span>
          </el-form-item>
          <el-form-item label="部署目录">
            <el-input
              v-model="config.installDir"
              :disabled="busy"
              placeholder="请输入目标主机上的绝对路径"
              aria-label="部署目录"
            />
            <p class="field-hint">服务文件将写入此目录；增量更新应选择现有部署目录。</p>
          </el-form-item>
          <el-form-item v-if="isRemote" label="连接检查">
            <div class="input-row wrap">
              <el-button :loading="testingSsh" :disabled="busy" @click="testConnection"> 测试连接 </el-button>
              <el-button :loading="settingKey" :disabled="busy" @click="setupAccess">
                配置免密登录
              </el-button>
              <span v-if="sshMessage" :class="sshOk ? 'text-success' : 'text-danger'">
                {{ sshMessage }}
              </span>
            </div>
          </el-form-item>
        </template>
      </el-form>
      <el-empty
        v-if="!connectionInventory.hosts.length"
        description="暂无可选主机，请先在主机管理中添加"
        :image-size="72"
      />
      <div class="step-footer">
        <span class="field-hint">{{ selectedHost ? `当前目标：${targetLabel}` : '请先选择主机' }}</span>
        <el-button type="primary" :disabled="!hostReady || busy" @click="nextStep">
          下一步：检测环境
          <el-icon class="el-icon--right"><ArrowRight /></el-icon>
        </el-button>
      </div>
    </div>

    <div v-if="activeStep === 1" class="step-card" v-loading="checkingEnv">
      <div class="section-heading">
        <div>
          <h3>检测目标环境</h3>
          <p class="desc">检查 {{ targetLabel }} 的系统资源、Docker、Compose 与端口占用。</p>
        </div>
        <el-button :loading="checkingEnv" :disabled="busy" @click="checkEnvironment"> 重新检测 </el-button>
      </div>
      <el-alert v-if="envError" :title="envError" type="error" :closable="false" show-icon />
      <template v-if="envData">
        <div class="env-metrics">
          <div class="metric-card">
            <span>操作系统 / 架构</span>
            <strong>{{ envData.os_release }} / {{ envData.arch }}</strong>
          </div>
          <div class="metric-card">
            <span>CPU 核心</span>
            <strong>{{ envData.cpu_cores }} 核</strong>
          </div>
          <div class="metric-card">
            <span>物理内存</span>
            <strong>{{ envData.total_memory_mb }} MB</strong>
          </div>
          <div class="metric-card">
            <span>可用磁盘</span>
            <strong>{{ envData.disk_available_gb }} GB</strong>
          </div>
        </div>
        <div class="status-row">
          <el-tag :type="envData.docker_running ? 'success' : 'danger'">
            {{ envData.docker_running ? 'Docker 正常运行' : 'Docker 未就绪' }}
          </el-tag>
          <el-tag :type="envData.compose_installed ? 'success' : 'danger'">
            {{ envData.compose_installed ? 'Compose 已安装' : 'Compose 未安装' }}
          </el-tag>
        </div>
        <ul v-if="envData.issues?.length" class="issue-list">
          <li v-for="issue in envData.issues" :key="issue">
            {{ safeText(issue) }}
          </li>
        </ul>
        <el-collapse v-if="!envData.docker_running" class="optional-panel">
          <el-collapse-item title="安装离线 Docker 引擎" name="docker">
            <p class="field-hint">
              请填写{{ isRemote ? '目标主机' : '本机' }}上的离线介质目录，目录中需包含 Docker 安装包。
            </p>
            <div class="input-row">
              <el-input v-model="config.mediaDir" placeholder="离线介质目录" :disabled="busy" />
              <el-button
                :loading="installingDocker"
                :disabled="!config.mediaDir || busy"
                @click="installDocker"
              >
                安装并重新检测
              </el-button>
            </div>
          </el-collapse-item>
        </el-collapse>
        <el-table :data="envData.ports || []" size="small" empty-text="暂无需检查的端口">
          <el-table-column prop="port" label="端口" width="90" />
          <el-table-column prop="protocol" label="协议" width="80" />
          <el-table-column prop="service" label="服务" min-width="170" />
          <el-table-column label="状态" width="110">
            <template #default="{ row }">
              <el-tag :type="row.available ? 'success' : 'warning'" size="small">
                {{ row.available ? '空闲' : '占用中' }}
              </el-tag>
            </template>
          </el-table-column>
        </el-table>
      </template>
      <div class="step-footer">
        <el-button :disabled="busy" @click="previousStep"> 上一步 </el-button>
        <el-button type="primary" :disabled="!environmentReady || busy" @click="nextStep">
          下一步：选择制品
          <el-icon class="el-icon--right"><ArrowRight /></el-icon>
        </el-button>
      </div>
    </div>

    <div v-show="activeStep === 2 || activeStep === 3" class="step-card">
      <h3>{{ activeStep === 2 ? '选择部署制品' : '解压并校验制品' }}</h3>
      <p class="desc">
        {{
          activeStep === 2
            ? '选择需要部署的版本，支持在线制品源和本地交付包。'
            : '通过检查后，再读取服务清单并核对目标主机。'
        }}
      </p>
      <WizardArtifactPicker
        ref="artifactPicker"
        :connection="config"
        :stage="activeStep === 2 ? 'select' : 'prepare'"
        :disabled="executing"
        @chosen="artifactChosen"
        @selected="artifactPrepared"
        @busy-change="artifactBusy = $event"
        @go-artifacts="emit('go-artifacts')"
      />
      <div class="step-footer">
        <el-button :disabled="busy" @click="previousStep"> 上一步 </el-button>
        <el-button
          type="primary"
          :disabled="busy || (activeStep === 2 ? !candidate : !preparedBundle)"
          @click="nextStep"
        >
          {{ activeStep === 2 ? '下一步：解压制品' : '下一步：清单比对' }}
          <el-icon class="el-icon--right"><ArrowRight /></el-icon>
        </el-button>
      </div>
    </div>

    <div v-if="activeStep === 4" class="step-card" v-loading="checkingReview">
      <div class="section-heading">
        <div>
          <h3>核对服务清单与镜像</h3>
          <p class="desc">对比所选制品与 {{ targetLabel }} 上的现有部署，确认所需镜像是否已具备。</p>
        </div>
        <el-button :loading="checkingReview" :disabled="busy" @click="loadReview"> 重新比对 </el-button>
      </div>
      <el-alert v-if="reviewError" :title="reviewError" type="error" :closable="false" show-icon />
      <template v-if="review">
        <div class="status-row">
          <el-tag :type="review.hasDeployment ? 'primary' : 'info'">
            {{ review.hasDeployment ? '检测到现有部署' : '未检测到现有部署' }}
          </el-tag>
          <span>制品版本 {{ preparedBundle?.version || '未标记' }}</span>
          <span>{{ review.services?.filter((s) => s.change !== 'removed').length }} 个服务</span>
        </div>
        <p class="field-hint">部署清单：{{ review.composeFile }}</p>
        <el-tabs v-model="reviewTab">
          <el-tab-pane label="服务清单比对" name="services">
            <el-table :data="review.services || []" size="small" empty-text="未读取到服务清单">
              <el-table-column prop="name" label="服务" min-width="140" />
              <el-table-column label="制品镜像" min-width="240">
                <template #default="{ row }">
                  <code>{{ row.image || '未声明' }}</code>
                </template>
              </el-table-column>
              <el-table-column label="当前镜像" min-width="220">
                <template #default="{ row }">
                  <code>{{ row.currentImage || '未部署' }}</code>
                </template>
              </el-table-column>
              <el-table-column label="变化" width="110">
                <template #default="{ row }">
                  <el-tag size="small" :type="row.change === 'unchanged' ? 'info' : 'warning'">
                    {{ changeLabel(row.change) }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column label="容器状态" width="110">
                <template #default="{ row }">{{ stateLabel(row.containerState) }}</template>
              </el-table-column>
            </el-table>
          </el-tab-pane>
          <el-tab-pane label="镜像核对" name="images">
            <el-table :data="review.images || []" size="small" empty-text="清单中没有镜像">
              <el-table-column label="所需镜像" min-width="280">
                <template #default="{ row }">
                  <code>{{ row.imageRef }}</code>
                </template>
              </el-table-column>
              <el-table-column label="服务" min-width="150">
                <template #default="{ row }">{{ row.services?.join('、') }}</template>
              </el-table-column>
              <el-table-column label="目标主机" width="115">
                <template #default="{ row }">
                  <el-tag size="small" :type="row.status === 'ready' ? 'success' : 'warning'">
                    {{ row.status === 'ready' ? '已具备' : row.status === 'unconfirmed' ? '未确认' : '缺失' }}
                  </el-tag>
                </template>
              </el-table-column>
            </el-table>
          </el-tab-pane>
        </el-tabs>
        <ul v-if="review.issues?.length" class="issue-list">
          <li v-for="issue in review.issues" :key="issue">
            {{ safeText(issue) }}
          </li>
        </ul>
        <div v-if="review.missingImages?.length" class="image-preparation">
          <h4>准备缺失镜像</h4>
          <p class="field-hint">
            选择本机的离线介质目录（含 images/），将其中的镜像包导入{{
              isRemote ? '目标主机' : '本机'
            }}。导入后会重新比对。
          </p>
          <div class="input-row wrap">
            <el-input
              v-model="imageMediaDir"
              :disabled="busy"
              :placeholder="preparedBundle?.dir || '本机离线介质目录'"
              aria-label="镜像介质目录"
            />
            <el-button :disabled="busy" @click="pickImageDirectory"> 选择目录 </el-button>
            <el-button type="primary" :loading="importingImages" :disabled="busy" @click="prepareImages">
              导入镜像并比对
            </el-button>
          </div>
        </div>
      </template>
      <div class="step-footer">
        <el-button :disabled="busy" @click="previousStep"> 上一步 </el-button>
        <el-button type="primary" :disabled="busy || !review?.ready" @click="nextStep">
          下一步：选择更新
          <el-icon class="el-icon--right"><ArrowRight /></el-icon>
        </el-button>
      </div>
    </div>

    <div v-if="activeStep === 5" class="step-card">
      <h3>选择部署操作</h3>
      <p class="desc">
        目标 {{ targetLabel }}，部署目录 {{ config.installDir }}，制品版本
        {{ preparedBundle?.version || '未标记' }}。
      </p>
      <el-radio-group
        v-model="deploymentAction"
        class="deployment-actions"
        :disabled="busy || Boolean(result?.success || result?.pending)"
      >
        <el-radio value="first" border :disabled="!review?.canFirstDeploy">
          <span class="action-title">首次部署</span>
          <span class="action-description">写入交付包并启动清单中的服务，适用于尚未部署的目标目录。</span>
        </el-radio>
        <el-radio value="incremental" border :disabled="!review?.canIncrementalUpdate">
          <span class="action-title">增量更新</span>
          <span class="action-description">备份并替换所选内容，重启受影响的服务，适用于已有部署。</span>
        </el-radio>
      </el-radio-group>
      <el-form v-if="deploymentAction === 'incremental'" label-width="110px" @submit.prevent>
        <el-form-item label="更新内容">
          <el-checkbox-group
            v-model="selectedComponents"
            :disabled="busy || Boolean(result?.success || result?.pending)"
          >
            <el-checkbox v-for="component in componentOptions" :key="component.key" :value="component.key">
              {{ component.label }}
            </el-checkbox>
          </el-checkbox-group>
        </el-form-item>
        <el-form-item label="更新前备份">
          <el-switch v-model="backup" :disabled="busy || Boolean(result?.success || result?.pending)" />
          <span class="field-hint backup-hint">保留被替换文件，便于需要时恢复。</span>
        </el-form-item>
      </el-form>
      <el-alert
        v-if="preflightPassed && !result"
        title="预检通过，可以开始执行。"
        type="success"
        :closable="false"
        show-icon
      />
      <el-alert v-if="operationError" :title="operationError" type="error" :closable="false" show-icon />
      <div v-if="executing || phase" class="execution-status" role="status" aria-live="polite">
        <el-icon v-if="executing" class="is-loading">
          <Loading />
        </el-icon>
        {{ phase }}
      </div>
      <div v-if="result" class="execution-result">
        <el-alert
          :title="
            result.success
              ? deploymentAction === 'first'
                ? '首次部署执行完成'
                : '增量更新执行完成'
              : result.pending
              ? '操作已执行，服务待确认'
              : '操作未完成'
          "
          :description="result.message"
          :type="result.success ? 'success' : result.pending ? 'warning' : 'error'"
          :closable="false"
          show-icon
        />
        <p v-if="result.backupDir" class="field-hint">备份目录：{{ result.backupDir }}</p>
        <el-button v-if="result.success || result.pending" type="primary" plain @click="emit('go-dashboard')">
          查看集群监控
        </el-button>
      </div>
      <el-collapse v-if="diagnostics" class="optional-panel">
        <el-collapse-item title="执行详情" name="details">
          <pre class="terminal-log">{{ diagnostics }}</pre>
        </el-collapse-item>
      </el-collapse>
      <div class="step-footer">
        <el-button :disabled="busy" @click="previousStep"> 返回清单比对 </el-button>
        <div class="input-row wrap">
          <el-button
            :loading="preflighting"
            :disabled="!canExecute || busy || Boolean(result?.success || result?.pending)"
            @click="runPreflight"
          >
            执行预检
          </el-button>
          <el-button
            type="primary"
            :loading="executing"
            :disabled="!canExecute || busy || Boolean(result?.success || result?.pending)"
            @click="executeDeployment"
          >
            {{ deploymentAction === 'first' ? '开始首次部署' : '开始增量更新' }}
          </el-button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { ElMessageBox } from 'element-plus';
import { open } from '@tauri-apps/plugin-dialog';
import { ArrowRight, Check, Loading } from '@element-plus/icons-vue';
import WizardArtifactPicker from '../components/WizardArtifactPicker.vue';
import { connectionInventory, useManagedHost } from '../connection.js';
import {
  checkLocalEnv,
  checkRemoteEnv,
  installOfflineDockerEngine,
  installRemoteDockerEngine,
  dockerComposeAction,
  remoteDockerComposeAction,
  installBundle,
  inspectBundlePlan,
  inspectDeploymentReview,
  isDesktopApp,
  testManagedHostConnection,
  setupManagedHostAccess,
  scanLocalImageBundle,
  loadOfflineImages,
  uploadRemoteImageBundle,
  importRemoteImageBundle,
  updateRemoteDeployment,
  updateLocalDeployment
} from '../api/tauri.js';

const props = defineProps({ connection: { type: Object, required: true } });
const emit = defineEmits(['go-dashboard', 'go-images', 'go-artifacts', 'go-hosts', 'busy-change']);
const config = props.connection;
const steps = ['选择主机', '检测环境', '选择制品', '解压制品', '清单比对', '选择更新'];
const activeStep = ref(0);
const selectedHost = computed(() =>
  connectionInventory.hosts.find((host) => host.id === config.activeHostId)
);
const isRemote = computed(() => selectedHost.value?.kind !== 'local');
const targetLabel = computed(() =>
  selectedHost.value
    ? isRemote.value
      ? `${selectedHost.value.name}（${config.remoteHost}）`
      : selectedHost.value.name === '本机'
      ? '本机'
      : `${selectedHost.value.name}（本机）`
    : '未选择主机'
);
const hostReady = computed(() =>
  Boolean(
    selectedHost.value &&
      validInstallDir(config.installDir) &&
      (!isRemote.value || (config.remoteHost && config.remoteUser))
  )
);
const selectingHost = ref(false),
  testingSsh = ref(false),
  settingKey = ref(false),
  sshMessage = ref(''),
  sshOk = ref(false);
const envData = ref(null),
  envError = ref(''),
  checkingEnv = ref(false),
  installingDocker = ref(false);
const artifactPicker = ref(null),
  candidate = ref(null),
  preparedBundle = ref(null),
  artifactBusy = ref(false);
const review = ref(null),
  reviewError = ref(''),
  checkingReview = ref(false),
  reviewTab = ref('services');
const bundlePlan = ref(null),
  imageMediaDir = ref(''),
  importingImages = ref(false);
const deploymentAction = ref('first'),
  selectedComponents = ref([]),
  backup = ref(true);
const executing = ref(false),
  preflighting = ref(false),
  preflightPassed = ref(false),
  phase = ref(''),
  operationError = ref(''),
  diagnostics = ref(''),
  result = ref(null);
const busy = computed(
  () =>
    selectingHost.value ||
    testingSsh.value ||
    settingKey.value ||
    checkingEnv.value ||
    installingDocker.value ||
    artifactBusy.value ||
    checkingReview.value ||
    importingImages.value ||
    executing.value ||
    preflighting.value
);
const environmentReady = computed(() =>
  Boolean(envData.value?.docker_running && envData.value?.compose_installed && !envError.value)
);
const componentLabels = {
  image: '镜像',
  backend: '后端服务',
  gateway: '网关服务',
  frontend: '前端页面',
  config: '部署配置'
};
const componentOptions = computed(() => {
  const declared = bundlePlan.value?.components || [];
  const keys =
    bundlePlan.value?.mode === 'image'
      ? ['image', 'config']
      : declared.length
      ? declared
      : ['backend', 'gateway', 'frontend', 'config'];
  return [...new Set(keys)]
    .filter((key) => componentLabels[key] && (bundlePlan.value?.mode !== 'file' || key !== 'image'))
    .map((key) => ({ key, label: componentLabels[key] }));
});
const canExecute = computed(
  () =>
    hostReady.value &&
    environmentReady.value &&
    preparedBundle.value &&
    review.value?.ready &&
    (deploymentAction.value === 'first'
      ? review.value.canFirstDeploy
      : review.value.canIncrementalUpdate && selectedComponents.value.length > 0)
);
let revision = 0,
  envRevision = 0,
  disposed = false;
watch(busy, (value) => emit('busy-change', value), { flush: 'sync' });
onBeforeUnmount(() => {
  disposed = true;
  revision += 1;
  envRevision += 1;
  emit('busy-change', false);
});
function validInstallDir(value) {
  const path = String(value || '').trim();
  return (
    !/[\n\r\0]/.test(path) &&
    path !== '/' &&
    path !== '\\' &&
    (/^\//.test(path) || (!isRemote.value && /^[A-Za-z]:[\\/].+/.test(path)))
  );
}
function safeText(value) {
  return String(value || '')
    .replace(/-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g, '[私钥已隐藏]')
    .replace(/\b(?:https?|ssh):\/\/[^\s]+/gi, '[地址已隐藏]')
    .replace(
      /((?:["']?)[\w-]*(?:password|passwd|token|secret|authorization)["']?\s*[:=]\s*)(?:"(?:\\.|[^"])*"|'(?:\\.|[^'])*'|[^\s,;]+)/gi,
      '$1[已隐藏]'
    )
    .replace(/((?:--password|--token)\s+)(?:"[^"]*"|'[^']*'|\S+)/gi, '$1[已隐藏]')
    .replace(/(Bearer\s+)[\w.-]+/gi, '$1[已隐藏]');
}
function failureText(error, action) {
  const text = String(error || '');
  if (/权限|denied|认证|authentication|publickey/i.test(text))
    return `${action}失败，请检查主机登录权限和凭证后重试。`;
  if (/Docker|docker|Compose|compose/.test(text))
    return `${action}失败，请确认 Docker 和 Compose 正常运行，并检查服务配置。`;
  if (/镜像|image/i.test(text)) return `${action}失败，请补齐所需镜像后重新比对。`;
  if (/目录|文件|路径|bundle|交付包/.test(text)) return `${action}失败，请检查交付包与部署目录后重试。`;
  return `${action}失败，请检查目标主机连接后重试。`;
}
const changeLabel = (value) =>
  ({ added: '新增', changed: '有变化', unchanged: '一致', removed: '制品中移除' }[value] || '未确认');
const stateLabel = (value) =>
  ({
    running: '运行中',
    exited: '已停止',
    created: '已创建',
    restarting: '重启中',
    not_created: '未创建',
    missing: '未创建'
  }[value] || '未运行');
const target = () =>
  isRemote.value
    ? {
        host: config.remoteHost,
        port: config.remotePort,
        user: config.remoteUser,
        keyPath: config.remoteKey || null
      }
    : null;
function desktopGuard() {
  if (!isDesktopApp()) throw new Error('当前环境暂不可用，请在桌面应用中操作。');
}
function clearExecution() {
  preflightPassed.value = false;
  result.value = null;
  phase.value = '';
  operationError.value = '';
  diagnostics.value = '';
}
function clearReview() {
  revision += 1;
  review.value = null;
  reviewError.value = '';
  bundlePlan.value = null;
  clearExecution();
}
watch(
  () => [
    config.activeHostId,
    config.mode,
    config.remoteHost,
    config.remotePort,
    config.remoteUser,
    config.remoteKey,
    config.installDir
  ],
  () => {
    if (executing.value) return;
    envRevision += 1;
    clearReview();
    envData.value = null;
    envError.value = '';
    sshMessage.value = '';
    if (activeStep.value > 0) activeStep.value = 0;
    config.deployDir = config.installDir;
  }
);
watch(() => [deploymentAction.value, selectedComponents.value.join(','), backup.value], clearExecution);
function artifactChosen(value) {
  candidate.value = value;
  preparedBundle.value = null;
  clearReview();
}
function artifactPrepared(value) {
  preparedBundle.value = value;
  clearReview();
}
async function selectHost(id) {
  if (busy.value) return;
  selectingHost.value = true;
  try {
    await useManagedHost(id);
  } catch (error) {
    sshOk.value = false;
    sshMessage.value = failureText(error, '切换主机');
  } finally {
    selectingHost.value = false;
  }
}
async function testConnection() {
  testingSsh.value = true;
  sshMessage.value = '';
  try {
    desktopGuard();
    const response = await testManagedHostConnection(config.activeHostId);
    sshOk.value = Boolean(response.success);
    sshMessage.value = response.success ? '连接正常' : '连接未通过，请检查凭证与网络。';
  } catch (error) {
    sshOk.value = false;
    sshMessage.value = failureText(error, '连接检查');
  } finally {
    testingSsh.value = false;
  }
}
async function setupAccess() {
  try {
    await ElMessageBox.confirm(
      `将为 ${targetLabel.value} 配置免密登录，使用该主机保存的凭证。`,
      '确认配置免密登录',
      { type: 'warning', confirmButtonText: '开始配置', cancelButtonText: '取消' }
    );
  } catch {
    return;
  }
  settingKey.value = true;
  try {
    desktopGuard();
    const response = await setupManagedHostAccess(config.activeHostId);
    sshOk.value = Boolean(response.success);
    sshMessage.value = response.success ? '免密登录已配置' : '配置未完成，请检查凭证后重试。';
  } catch (error) {
    sshOk.value = false;
    sshMessage.value = failureText(error, '配置免密登录');
  } finally {
    settingKey.value = false;
  }
}
async function checkEnvironment() {
  const current = ++envRevision;
  checkingEnv.value = true;
  envError.value = '';
  envData.value = null;
  review.value = null;
  clearExecution();
  try {
    desktopGuard();
    const response = isRemote.value
      ? await checkRemoteEnv(
          config.remoteHost,
          config.remotePort,
          config.remoteUser,
          config.remoteKey || null
        )
      : await checkLocalEnv(config.installDir);
    if (current === envRevision && !disposed) envData.value = response;
  } catch (error) {
    if (current === envRevision) envError.value = failureText(error, '环境检测');
  } finally {
    checkingEnv.value = false;
  }
}
async function installDocker() {
  try {
    await ElMessageBox.confirm(
      `将在 ${targetLabel.value} 安装并启动 Docker 引擎，可能修改系统服务配置。`,
      '确认安装 Docker',
      { type: 'warning', confirmButtonText: '安装', cancelButtonText: '取消' }
    );
  } catch {
    return;
  }
  installingDocker.value = true;
  try {
    desktopGuard();
    const response = isRemote.value
      ? await installRemoteDockerEngine(
          config.remoteHost,
          config.remotePort,
          config.remoteUser,
          config.remoteKey || null,
          config.mediaDir
        )
      : await installOfflineDockerEngine(config.mediaDir);
    if (!response.success) throw new Error(response.error_message || 'Docker 安装失败');
    await checkEnvironment();
  } catch (error) {
    envError.value = failureText(error, '安装 Docker');
  } finally {
    installingDocker.value = false;
  }
}
async function nextStep() {
  if (busy.value) return;
  if (activeStep.value === 0 && hostReady.value) {
    activeStep.value = 1;
    await checkEnvironment();
  } else if (activeStep.value === 1 && environmentReady.value) activeStep.value = 2;
  else if (activeStep.value === 2 && candidate.value) activeStep.value = 3;
  else if (activeStep.value === 3 && preparedBundle.value) {
    activeStep.value = 4;
    await loadReview();
  } else if (activeStep.value === 4 && review.value?.ready) {
    deploymentAction.value = review.value.hasDeployment ? 'incremental' : 'first';
    selectedComponents.value = componentOptions.value.map((component) => component.key);
    activeStep.value = 5;
  }
}
async function previousStep() {
  if (busy.value || activeStep.value === 0) return;
  activeStep.value -= 1;
  if (activeStep.value === 4) await loadReview();
}
async function loadReview() {
  if (!preparedBundle.value) return;
  const current = ++revision;
  checkingReview.value = true;
  review.value = null;
  reviewError.value = '';
  clearExecution();
  try {
    desktopGuard();
    const [checked, plan] = await Promise.all([
      inspectDeploymentReview(preparedBundle.value.dir, config.installDir, target(), imageMediaDir.value),
      inspectBundlePlan(preparedBundle.value.dir, target())
    ]);
    if (current !== revision || disposed) return;
    review.value = checked;
    bundlePlan.value = plan;
  } catch (error) {
    if (current === revision) reviewError.value = failureText(error, '清单比对');
  } finally {
    checkingReview.value = false;
  }
}
async function pickImageDirectory() {
  try {
    desktopGuard();
    const directory = await open({
      directory: true,
      multiple: false,
      title: '选择含 images/ 的离线介质目录'
    });
    if (typeof directory === 'string') imageMediaDir.value = directory;
  } catch (error) {
    reviewError.value = failureText(error, '选择镜像目录');
  }
}
async function prepareImages() {
  const root = imageMediaDir.value.trim() || preparedBundle.value?.dir;
  importingImages.value = true;
  reviewError.value = '';
  try {
    desktopGuard();
    const scan = await scanLocalImageBundle(root);
    const files = (scan.files || []).map((file) => file.name);
    if (!files.length) throw new Error('镜像目录中没有离线包');
    try {
      await ElMessageBox.confirm(
        `将向 ${targetLabel.value} 导入 ${files.length} 个镜像包。${
          isRemote.value ? '镜像包会先上传到部署目录。' : ''
        }此操作会占用磁盘空间。`,
        '确认导入镜像',
        { type: 'warning', confirmButtonText: '导入', cancelButtonText: '取消' }
      );
    } catch {
      return;
    }
    if (isRemote.value) {
      const remoteDir = `${config.installDir.replace(/\/+$/, '')}/bundle-images-${Date.now()}`;
      await uploadRemoteImageBundle(
        config.remoteHost,
        config.remotePort,
        config.remoteUser,
        config.remoteKey || null,
        root,
        remoteDir,
        files,
        false
      );
      await importRemoteImageBundle(
        config.remoteHost,
        config.remotePort,
        config.remoteUser,
        config.remoteKey || null,
        remoteDir,
        files
      );
    } else {
      await loadOfflineImages(root);
    }
    await loadReview();
  } catch (error) {
    reviewError.value = failureText(error, '导入镜像');
  } finally {
    importingImages.value = false;
  }
}
function executionContext() {
  return Object.freeze({
    remote: isRemote.value,
    host: config.remoteHost,
    port: config.remotePort,
    user: config.remoteUser,
    keyPath: config.remoteKey || null,
    bundleDir: preparedBundle.value.dir,
    installDir: config.installDir,
    imageDir: imageMediaDir.value,
    version: preparedBundle.value.version,
    components: Object.freeze([...selectedComponents.value]),
    backup: backup.value,
    action: deploymentAction.value,
    label: targetLabel.value
  });
}
const contextTarget = (context) =>
  context.remote
    ? { host: context.host, port: context.port, user: context.user, keyPath: context.keyPath }
    : null;
async function invokeIncremental(context, dryRun) {
  return context.remote
    ? await updateRemoteDeployment(
        context.host,
        context.port,
        context.user,
        context.keyPath,
        context.bundleDir,
        context.installDir,
        [...context.components],
        context.backup,
        dryRun
      )
    : await updateLocalDeployment(
        context.bundleDir,
        context.installDir,
        [...context.components],
        context.backup,
        dryRun
      );
}
async function preflight(context) {
  preflightPassed.value = false;
  desktopGuard();
  const checked = await inspectDeploymentReview(
    context.bundleDir,
    context.installDir,
    contextTarget(context),
    context.imageDir
  );
  review.value = checked;
  if (
    !checked.ready ||
    (context.action === 'first' ? !checked.canFirstDeploy : !checked.canIncrementalUpdate)
  )
    throw new Error('清单、镜像或目标部署状态已变化，请返回重新比对');
  if (context.action === 'incremental') {
    const response = await invokeIncremental(context, true);
    if (!response.success) throw new Error(response.error || '增量更新预检未通过');
    diagnostics.value = safeText(response.output);
  }
  preflightPassed.value = true;
}
async function runPreflight() {
  if (!canExecute.value || busy.value) return;
  preflighting.value = true;
  operationError.value = '';
  preflightPassed.value = false;
  try {
    await preflight(executionContext());
  } catch (error) {
    operationError.value = failureText(error, '预检');
    diagnostics.value = safeText(error);
  } finally {
    preflighting.value = false;
  }
}
async function executeDeployment() {
  if (!canExecute.value || busy.value) return;
  const context = executionContext();
  preflighting.value = true;
  operationError.value = '';
  result.value = null;
  try {
    await preflight(context);
  } catch (error) {
    operationError.value = failureText(error, '预检');
    diagnostics.value = safeText(error);
    return;
  } finally {
    preflighting.value = false;
  }
  const action = context.action === 'first' ? '首次部署' : '增量更新';
  const scope =
    context.action === 'first'
      ? '写入交付包并启动全部清单服务'
      : `替换${context.components.map((key) => componentLabels[key]).join('、')}并重启受影响的服务${
          context.backup ? '，更新前保留备份' : '，本次不备份'
        }`;
  executing.value = true;
  try {
    try {
      await ElMessageBox.confirm(
        `目标：${context.label}；目录：${context.installDir}；版本：${
          context.version || '未标记'
        }。将${scope}。是否继续？`,
        `确认${action}`,
        {
          type: 'warning',
          confirmButtonText: `开始${action}`,
          cancelButtonText: '取消',
          closeOnClickModal: false
        }
      );
    } catch {
      return;
    }
    phase.value = `正在${action}，请等待执行结果`;
    let response;
    if (context.action === 'incremental') {
      response = await invokeIncremental(context, false);
      if (!response.success) {
        result.value = { success: false, backupDir: response.backup_dir };
        diagnostics.value = safeText(response.output);
        throw new Error(response.error || '更新未完成');
      }
    } else {
      phase.value = '正在写入交付包';
      const installed = await installBundle(
        context.remote ? 'remote' : 'local',
        context.host,
        context.port,
        context.user,
        context.keyPath,
        context.bundleDir,
        context.installDir
      );
      if (!installed.success) throw new Error(installed.error || '交付包写入失败');
      diagnostics.value = safeText(installed.output);
      phase.value = '正在启动清单中的服务';
      response = context.remote
        ? await remoteDockerComposeAction(
            context.host,
            context.port,
            context.user,
            context.keyPath,
            context.installDir,
            'up'
          )
        : await dockerComposeAction(context.installDir, 'up');
      if (!response.success) throw new Error(response.error || '服务启动失败');
    }
    diagnostics.value += `\n${safeText(response.output)}`;
    phase.value = '正在检查服务运行状态';
    const verified = await inspectDeploymentReview(
      context.bundleDir,
      context.installDir,
      contextTarget(context),
      context.imageDir
    );
    const services = (verified.services || []).filter((service) => service.change !== 'removed');
    const ready =
      services.length > 0 &&
      verified.targetChecked &&
      services.every(
        (service) =>
          service.containerState === 'running' && !['unhealthy', 'starting'].includes(service.containerHealth)
      );
    result.value = {
      success: ready,
      pending: !ready,
      backupDir: response.backup_dir,
      message: ready
        ? '服务已运行，可前往集群监控检查业务状态。'
        : '执行命令已完成，部分服务的就绪状态仍需确认，请前往集群监控检查。'
    };
    phase.value = '';
  } catch (error) {
    result.value = {
      ...result.value,
      success: false,
      message: '执行未完成，请查看执行详情，处理后重新比对。'
    };
    operationError.value = failureText(error, action);
    diagnostics.value += `\n${safeText(error)}`;
    phase.value = '';
  } finally {
    executing.value = false;
  }
}
</script>

<style scoped>
.wizard-container {
  display: block;
}
.steps-header {
  margin-bottom: 20px;
  padding: 20px 22px;
}
.stepper {
  display: flex;
  align-items: center;
  justify-content: space-between;
  list-style: none;
  gap: 12px;
  overflow-x: auto;
}
.step {
  display: flex;
  align-items: center;
  gap: 9px;
  color: var(--text-muted);
  white-space: nowrap;
  font-size: 12px;
}
.step:not(:last-child)::after {
  content: '';
  width: clamp(8px, 2.4vw, 46px);
  height: 2px;
  background: var(--border-color);
  margin-left: 9px;
}
.step-node {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 29px;
  height: 29px;
  flex: none;
  border: 1px solid var(--border-strong);
  border-radius: 50%;
  background: white;
  font-size: 12px;
}
.step.active {
  color: var(--text-main);
  font-weight: 600;
}
.step.active .step-node {
  background: var(--primary-color);
  border-color: var(--primary-color);
  color: white;
  box-shadow: 0 0 0 4px var(--primary-ring);
}
.step.done .step-node {
  color: var(--success-color);
  border-color: var(--success-color);
  background: var(--success-soft);
}
.step-card {
  padding: 28px;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
}
h3 {
  font-size: 17px;
  margin-bottom: 7px;
}
.desc {
  color: var(--text-muted);
  font-size: 13px;
  line-height: 1.65;
  margin-bottom: 22px;
}
.host-form {
  margin-top: 26px;
  max-width: 850px;
}
.input-row,
.status-row,
.section-heading {
  display: flex;
  align-items: center;
  gap: 12px;
}
.input-row {
  width: 100%;
}
.input-row .el-select,
.input-row .el-input {
  flex: 1;
  min-width: 150px;
}
.wrap {
  flex-wrap: wrap;
}
.section-heading {
  justify-content: space-between;
  align-items: flex-start;
}
.step-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-top: 22px;
  margin-top: 26px;
  border-top: 1px solid var(--border-color);
}
.step-footer > .input-row {
  width: auto;
}
.field-hint {
  width: 100%;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.7;
  margin-top: 7px;
  overflow-wrap: anywhere;
}
.env-metrics {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 12px;
  margin: 22px 0;
}
.metric-card {
  padding: 15px;
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.metric-card span {
  color: var(--text-muted);
  font-size: 12px;
}
.metric-card strong {
  font-size: 14px;
  word-break: break-word;
}
.status-row {
  flex-wrap: wrap;
  margin: 18px 0;
  color: var(--text-secondary);
  font-size: 12px;
}
.issue-list {
  padding: 14px 14px 14px 30px;
  margin: 16px 0;
  color: var(--warning-color);
  background: var(--warning-soft);
  border-radius: var(--radius-sm);
  font-size: 12px;
  line-height: 1.8;
}
.optional-panel,
.execution-result,
.execution-status {
  margin-top: 20px;
}
.optional-panel .input-row {
  margin: 12px 0;
}
.image-preparation {
  padding: 18px;
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  margin-top: 20px;
}
.image-preparation .input-row {
  margin-top: 14px;
}
code {
  font-size: 12px;
  overflow-wrap: anywhere;
}
.deployment-actions {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
  width: 100%;
  margin: 10px 0 22px;
}
.deployment-actions :deep(.el-radio) {
  margin: 0;
  padding: 22px 18px;
  height: auto;
  white-space: normal;
  align-items: flex-start;
}
.deployment-actions :deep(.el-radio__input) {
  margin-top: 4px;
}
.deployment-actions :deep(.el-radio__label) {
  white-space: normal;
}
.action-title {
  display: block;
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 9px;
}
.action-description {
  display: block;
  font-size: 12px;
  line-height: 1.7;
  color: var(--text-muted);
}
.backup-hint {
  width: auto;
  margin: 0 0 0 12px;
}
.execution-status {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--text-secondary);
}
.execution-result > .el-button {
  margin-top: 16px;
}
.terminal-log {
  padding: 16px;
  color: #d7dde8;
  background: #0b1220;
  border-radius: var(--radius-sm);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 340px;
  overflow: auto;
  font-size: 12px;
  line-height: 1.65;
}
@media (max-width: 1100px) {
  .env-metrics {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .step:not(:last-child)::after {
    width: 10px;
    margin-left: 2px;
  }
  .steps-header {
    padding: 18px 14px;
  }
  .stepper {
    gap: 8px;
  }
}
@media (max-width: 760px) {
  .step-card {
    padding: 20px;
  }
  .deployment-actions {
    grid-template-columns: 1fr;
  }
  .step-footer,
  .section-heading {
    flex-wrap: wrap;
  }
}
</style>
