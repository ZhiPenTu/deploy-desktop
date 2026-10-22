<template>
  <div class="step-card">
    <h3>选择部署操作</h3>
    <p class="desc">
      目标 {{ targetLabel }}，部署目录 {{ installDir }}，制品版本 {{ version || '未标记' }}。
    </p>
    <el-radio-group
      :model-value="action"
      class="deployment-actions"
      :disabled="locked"
      @update:model-value="(value) => emit('update:action', value)"
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
    <el-form v-if="action === 'incremental'" label-width="110px" @submit.prevent>
      <el-form-item label="更新内容">
        <el-checkbox-group
          :model-value="components"
          :disabled="locked"
          @update:model-value="(value) => emit('update:components', value)"
        >
          <el-checkbox v-for="component in componentOptions" :key="component.key" :value="component.key">
            {{ component.label }}
          </el-checkbox>
        </el-checkbox-group>
      </el-form-item>
      <el-form-item label="更新前备份">
        <el-switch
          :model-value="backup"
          :disabled="locked"
          @update:model-value="(value) => emit('update:backup', value)"
        />
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
            ? action === 'first'
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
      <el-button :disabled="busy" @click="emit('previous')"> 返回清单比对 </el-button>
      <div class="input-row wrap">
        <el-button :loading="preflighting" :disabled="!canExecute || locked" @click="emit('preflight')">
          执行预检
        </el-button>
        <el-button type="primary" :loading="executing" :disabled="!canExecute || locked" @click="emit('execute')">
          {{ action === 'first' ? '开始首次部署' : '开始增量更新' }}
        </el-button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue';
import { Loading } from '@element-plus/icons-vue';

const props = defineProps({
  targetLabel: { type: String, default: '' },
  installDir: { type: String, default: '' },
  version: { type: String, default: '' },
  action: { type: String, default: 'first' },
  components: { type: Array, default: () => [] },
  componentOptions: { type: Array, default: () => [] },
  backup: { type: Boolean, default: true },
  review: { type: Object, default: null },
  busy: { type: Boolean, default: false },
  canExecute: { type: Boolean, default: false },
  preflighting: { type: Boolean, default: false },
  preflightPassed: { type: Boolean, default: false },
  executing: { type: Boolean, default: false },
  phase: { type: String, default: '' },
  operationError: { type: String, default: '' },
  progressPercent: { type: Number, default: 0 },
  progressStep: { type: String, default: '' },
  progressMessage: { type: String, default: '' },
  elapsedText: { type: String, default: '' },
  result: { type: Object, default: null },
  diagnostics: { type: String, default: '' }
});

const emit = defineEmits([
  'update:action',
  'update:components',
  'update:backup',
  'previous',
  'preflight',
  'execute',
  'go-dashboard'
]);

/** 执行已开始或已出结果时，操作选择不再接受修改。 */
const locked = computed(() => props.busy || Boolean(props.result?.success || props.result?.pending));
</script>

<style scoped>
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
.input-row {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
}
.wrap {
  flex-wrap: wrap;
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
.optional-panel,
.execution-result,
.execution-status {
  margin-top: 20px;
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
  .step-footer {
    flex-wrap: wrap;
  }
}
</style>
