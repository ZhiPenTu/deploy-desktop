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

    <ReviewStep
      v-if="activeStep === 4"
      :target-label="targetLabel"
      :checking-review="checkingReview"
      :busy="busy"
      :review-error="reviewError"
      :review="review"
      :version="preparedBundle?.version || ''"
      :bundle-dir="preparedBundle?.dir || ''"
      :is-remote="isRemote"
      :image-media-dir="imageMediaDir"
      v-model:image-source="imageSource"
      :image-phase="imagePhase"
      :image-progress="imageProgress"
      :image-diagnostics="imageDiagnostics"
      :image-result="imageResult"
      :importing-images="importingImages"
      :review-tab="reviewTab"
      @recheck="loadReview"
      @update:review-tab="reviewTab = $event"
      @update:image-media-dir="imageMediaDir = $event"
      @pick-image-directory="pickImageDirectory"
      @prepare-images="prepareImages"
      @previous="previousStep"
      @next="nextStep"
    />
    <ExecuteStep
      v-if="activeStep === 5"
      :target-label="targetLabel"
      :install-dir="config.installDir"
      :version="preparedBundle?.version || ''"
      :action="deploymentAction"
      :components="selectedComponents"
      :component-options="componentOptions"
      :backup="backup"
      :review="review"
      :busy="busy"
      :can-execute="canExecute"
      :preflighting="preflighting"
      :preflight-passed="preflightPassed"
      :executing="executing"
      :phase="phase"
      :operation-error="operationError"
      :progress-percent="progressPercent"
      :progress-step="progressStep"
      :progress-message="progressMessage"
      :elapsed-text="elapsedText"
      :result="result"
      :diagnostics="diagnostics"
      @update:action="deploymentAction = $event"
      @update:components="selectedComponents = $event"
      @update:backup="backup = $event"
      @previous="previousStep"
      @preflight="runPreflight"
      @execute="executeDeployment"
      @go-dashboard="emit('go-dashboard')"
    />
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { ElMessage } from 'element-plus';
import { ArrowRight } from '@element-plus/icons-vue';
import DeploymentStepper from '../components/DeploymentStepper.vue';
import DeployTargetStep from '../components/DeployTargetStep.vue';
import EnvironmentStep from '../components/EnvironmentStep.vue';
import ExecuteStep from '../components/ExecuteStep.vue';
import ReviewStep from '../components/ReviewStep.vue';
import WizardArtifactPicker from '../../artifacts/components/WizardArtifactPicker.vue';
import { useDeploymentWizard } from '../composables/useDeploymentWizard.js';
import { connectionInventory, switchActiveProject } from '../../connections/state.js';
import { projectState } from '../../../config/project.js';
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
  imageSource,
  imagePhase,
  imageProgress,
  imageDiagnostics,
  imageResult,
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
.step-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-top: 22px;
  margin-top: 26px;
  border-top: 1px solid var(--border-color);
}
@media (max-width: 760px) {
  .step-card {
    padding: 20px;
  }
  .step-footer {
    flex-wrap: wrap;
  }
}
</style>
