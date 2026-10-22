<template>
  <div class="page-view wizard-container">
    <div class="page-header">
      <div>
        <h2 class="page-title">部署向导</h2>
        <p class="page-subtitle">选择项目与制品，核对服务和镜像后执行首次部署或增量更新</p>
      </div>
    </div>
    <DeploymentStepper :steps="steps" :active-step="activeStep" />

    <DeployTargetStep
      v-if="activeStep === 0"
      :has-project="hasProject"
      :projects="projects"
      :selected-project-id="selectedProjectId"
      :switching-project="switchingProject"
      :busy="busy"
      :target-label="targetLabel"
      :install-dir="config.installDir"
      :deploy-dir="config.deployDir"
      :is-remote="isRemote"
      :inventory-error="connectionInventory.error"
      :testing-ssh="testingSsh"
      :setting-key="settingKey"
      :ssh-message="sshMessage"
      :ssh-ok="sshOk"
      :host-ready="hostReady"
      @go-projects="emit('go-projects')"
      @select-project="selectProject"
      @test-connection="testConnection"
      @setup-access="setupAccess"
      @next="nextStep"
    />

    <EnvironmentStep
      v-if="activeStep === 1"
      :target-label="targetLabel"
      :checking-env="checkingEnv"
      :busy="busy"
      :env-error="envError"
      :env-data="envData"
      :is-remote="isRemote"
      :media-dir="config.mediaDir"
      :installing-docker="installingDocker"
      :environment-ready="environmentReady"
      @recheck="checkEnvironment"
      @install-docker="installDocker"
      @update:media-dir="config.mediaDir = $event"
      @previous="previousStep"
      @next="nextStep"
    />

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
        :preset="presetBundle"
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
        <div class="section-heading compose-review-heading">
          <p class="field-hint">部署清单：{{ review.composeFile }}</p>
          <el-button :disabled="checkingReview" @click="openConfigDiff()">查看配置对比</el-button>
        </div>
        <el-alert
          v-if="review.hasDeployment && review.composeChanged"
          title="配置有变化，可继续选择更新内容；仅勾选“部署配置”时才会替换配置文件。"
          type="info"
          :closable="false"
          show-icon
        />
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
                  <el-button
                    v-if="row.change !== 'unchanged'"
                    link
                    type="warning"
                    :aria-label="`查看 ${row.name} 的配置差异`"
                    @click="openConfigDiff(row.name)"
                  >
                    {{ changeLabel(row.change) }}
                  </el-button>
                  <el-tag v-else size="small" type="info">
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
        <ConfigDiffDialog
          v-if="configDiffOpen"
          v-model="configDiffOpen"
          :review="review"
          :service-name="configDiffService"
          :version="preparedBundle?.version || ''"
        />
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
      <div v-if="executing" class="execution-progress">
        <el-progress :percentage="progressPercent" :stroke-width="8" />
        <p class="field-hint">
          当前步骤：{{ progressStep || '准备执行' }}
          <template v-if="progressMessage"> · {{ progressMessage }}</template>
          · 已用时 {{ elapsedText }}
        </p>
        <p class="field-hint">
          应用配置后的服务就绪检查最长约 3 分钟，请保持页面打开，完成或失败后都会显示结果。
        </p>
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
import { computed, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { ArrowRight, Loading } from '@element-plus/icons-vue';
import DeploymentStepper from '../components/DeploymentStepper.vue';
import DeployTargetStep from '../components/DeployTargetStep.vue';
import EnvironmentStep from '../components/EnvironmentStep.vue';
import ConfigDiffDialog from '../components/ConfigDiffDialog.vue';
import WizardArtifactPicker from '../../artifacts/components/WizardArtifactPicker.vue';
import { useDeploymentWizard } from '../composables/useDeploymentWizard.js';
import { connectionInventory, switchActiveProject } from '../../connections/state.js';
import { projectState } from '../../../config/project.js';
import { safeText } from '../domain/errors.js';
import { errorText } from '../../../shared/errors.js';

const props = defineProps({
  connection: { type: Object, required: true },
  presetBundle: { type: Object, default: null }
});
const emit = defineEmits(['go-dashboard', 'go-images', 'go-artifacts', 'go-projects', 'busy-change']);
const hasProject = computed(() => Boolean(projectState.project?.id));
const projects = computed(() => projectState.projects);
const selectedProjectId = computed(() => projectState.project?.id || '');
const switchingProject = ref(false);
const config = props.connection;
const configDiffOpen = ref(false);
const configDiffService = ref('');
function openConfigDiff(serviceName = '') {
  configDiffService.value = serviceName;
  configDiffOpen.value = true;
}

async function selectProject(id) {
  if (!id || id === selectedProjectId.value || switchingProject.value) return;
  switchingProject.value = true;
  try {
    await switchActiveProject(id);
    ElMessage.success(`已选择项目「${projectState.project?.name || ''}」`);
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    switchingProject.value = false;
  }
}
const {
  steps,
  activeStep,
  selectedHost,
  isRemote,
  targetLabel,
  hostReady,
  testingSsh,
  settingKey,
  sshMessage,
  sshOk,
  envData,
  envError,
  checkingEnv,
  installingDocker,
  artifactPicker,
  candidate,
  preparedBundle,
  artifactBusy,
  review,
  reviewError,
  checkingReview,
  reviewTab,
  imageMediaDir,
  importingImages,
  deploymentAction,
  selectedComponents,
  backup,
  executing,
  preflighting,
  preflightPassed,
  phase,
  operationError,
  diagnostics,
  result,
  busy,
  environmentReady,
  componentOptions,
  canExecute,
  changeLabel,
  stateLabel,
  artifactChosen,
  artifactPrepared,
  testConnection,
  setupAccess,
  checkEnvironment,
  installDocker,
  nextStep,
  previousStep,
  loadReview,
  pickImageDirectory,
  prepareImages,
  runPreflight,
  executeDeployment,
  progressStep,
  progressPercent,
  progressMessage,
  elapsedText
} = useDeploymentWizard(config, emit);

watch([review, activeStep], () => {
  configDiffOpen.value = false;
  configDiffService.value = '';
});
</script>

<style scoped>
.wizard-container {
  display: block;
}
.step-card {
  padding: 26px 28px;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
}
h3 {
  font-size: 16px;
  font-weight: 650;
  letter-spacing: -0.01em;
  margin-bottom: 6px;
  color: var(--text-main);
}
.desc {
  color: var(--text-muted);
  font-size: 12.5px;
  line-height: 1.6;
  margin-bottom: 22px;
}
.input-row,
.status-row,
.section-heading {
  display: flex;
  align-items: center;
  gap: 12px;
}
.compose-review-heading { align-items: center; margin-bottom: 12px; }
.compose-review-heading .field-hint { flex: 1; margin-top: 0; }
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
  line-height: 1.65;
  margin-top: 6px;
  overflow-wrap: anywhere;
}
.status-row {
  flex-wrap: wrap;
  margin: 16px 0;
  color: var(--text-secondary);
  font-size: 12.5px;
}
.issue-list {
  padding: 14px 18px 14px 34px;
  margin: 16px 0;
  color: var(--warning-color);
  background: var(--warning-soft);
  border: 1px solid rgba(245, 158, 11, 0.2);
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
  padding: 18px 20px;
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
  margin: 12px 0 24px;
}
.deployment-actions :deep(.el-radio) {
  margin: 0;
  padding: 20px 22px;
  height: auto;
  white-space: normal;
  align-items: flex-start;
  border-radius: var(--radius-md);
  border: 1px solid var(--border-color);
  background: var(--panel-bg);
  box-shadow: var(--shadow-card);
  transition: all 0.18s ease;
}
.deployment-actions :deep(.el-radio.is-bordered.is-checked) {
  border-color: var(--primary-color);
  background: var(--primary-soft);
  box-shadow: var(--shadow-raised);
}
.deployment-actions :deep(.el-radio__input) {
  margin-top: 4px;
}
.deployment-actions :deep(.el-radio__label) {
  white-space: normal;
}
.action-title {
  display: block;
  font-size: 14.5px;
  font-weight: 600;
  color: var(--text-main);
  margin-bottom: 6px;
}
.action-description {
  display: block;
  font-size: 12px;
  line-height: 1.65;
  color: var(--text-muted);
}
.backup-hint {
  width: auto;
  margin: 0 0 0 12px;
}
.execution-status {
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--primary-color);
  font-size: 13.5px;
  font-weight: 550;
  padding: 12px 16px;
  background: var(--primary-soft);
  border: 1px solid rgba(51, 92, 255, 0.2);
  border-radius: var(--radius-sm);
  margin-top: 18px;
}
.execution-progress {
  margin-top: 14px;
}
.execution-progress .field-hint {
  margin: 8px 0 0;
}
.execution-result > .el-button {
  margin-top: 16px;
}
.terminal-log {
  padding: 16px 18px;
  color: #e2e8f0;
  background: #0f172a;
  border: 1px solid #1e293b;
  border-radius: var(--radius-sm);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 360px;
  overflow-y: auto;
  font-family: ui-monospace, "SFMono-Regular", Menlo, Monaco, Consolas, monospace;
  font-size: 11.5px;
  line-height: 1.65;
  box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.4);
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
