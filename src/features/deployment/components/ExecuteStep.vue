<template>
  <div class="step-card">
    <h3>部署完整版本</h3>
    <p class="desc">
      目标 {{ targetLabel }}，部署目录 {{ installDir }}，制品版本 {{ version || '未标记' }}。
      本次会部署交付包声明的全部服务和配置，不再进行部分组件更新。
    </p>
    <el-alert
      v-if="transactionBlocked"
      :title="
        transactionNeedsManual
          ? '上次自动恢复未完成或恢复被中断，处理完成前不能开始新部署。'
          : '检测到未完成的部署事务，处理完成前不能开始新部署。'
      "
      :description="
        transactionNeedsManual
          ? `现场已保留：${transaction?.snapshotDir || '事务快照目录'}。请按恢复错误排查后人工恢复，系统不会重复自动恢复。`
          : transaction?.error
          ? `失败原因：${transaction.error}`
          : '可以恢复部署前的版本和业务数据。'
      "
      type="error"
      :closable="false"
      show-icon
    />
    <el-alert
      v-if="artifactSelectionIssue"
      :title="artifactSelectionIssue"
      type="warning"
      :closable="false"
      show-icon
    />

    <div class="deployment-summary">
      <div class="summary-item">
        <span class="summary-label">部署方式</span>
        <strong>完整版本部署</strong>
      </div>
      <div class="summary-item">
        <span class="summary-label">服务</span>
        <strong>{{ services.length }} 个</strong>
      </div>
      <div class="summary-item">
        <span class="summary-label">停机</span>
        <strong>短暂停机，按数据量估算备份与恢复耗时</strong>
      </div>
      <div class="summary-item">
        <span class="summary-label">失败处理</span>
        <strong>自动恢复上一版本</strong>
      </div>
      <div class="summary-item">
        <span class="summary-label">备份空间</span>
        <strong>{{ backupEstimate }}</strong>
      </div>
    </div>

    <el-form label-width="110px" class="deployment-detail" @submit.prevent>
      <el-form-item label="受影响服务">
        <div class="tag-row">
          <el-tag v-for="service in services" :key="service.key" type="info">
            {{ service.label || service.key }}
          </el-tag>
          <span v-if="!services.length" class="field-hint">未读取到服务清单</span>
        </div>
      </el-form-item>
      <el-form-item label="保留数据">
        <div class="tag-row">
          <el-tag v-for="volume in dataVolumes" :key="volume.name" type="success">
            {{ volume.name }}
          </el-tag>
          <el-tag v-for="path in dataPaths" :key="path.path" type="success">
            {{ path.path }}
          </el-tag>
          <span v-if="!dataVolumes.length && !dataPaths.length" class="field-hint">
            交付包未声明数据卷
          </span>
        </div>
      </el-form-item>
    </el-form>
    <el-alert
      title="部署失败时会停止新版本、恢复部署前的受管文件与数据卷，并重新启动上一版本。失败期间的新写入会一并撤回。"
      type="info"
      :closable="false"
      show-icon
    />

    <el-alert
      v-if="preflightPassed && !result"
      title="预检通过，可以开始完整版本部署。"
      :description="preflightMessage"
      type="success"
      :closable="false"
      show-icon
      class="spaced"
    />
    <el-alert
      v-if="operationError"
      :title="operationError"
      type="error"
      :closable="false"
      show-icon
      class="spaced"
    />
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
        数据卷备份与恢复可能持续较长时间，请保持页面打开。连接中断不会终止目标机上的部署事务。
      </p>
    </div>
    <div v-if="result" class="execution-result">
      <el-alert
        :title="resultTitle"
        :description="result.message"
        :type="resultTone"
        :closable="false"
        show-icon
      />
      <p v-if="result.snapshotDir && !result.success" class="field-hint">
        事务快照：{{ result.snapshotDir }}
      </p>
      <el-button
        v-if="result.success"
        type="primary"
        plain
        @click="emit('go-dashboard')"
      >
        查看集群监控
      </el-button>
      <el-button
        v-if="transactionBlocked && !transactionNeedsManual"
        type="primary"
        :loading="recoveringTransaction"
        @click="emit('recover')"
      >
        恢复上一版本
      </el-button>
    </div>
    <el-collapse v-if="diagnostics" class="optional-panel">
      <el-collapse-item title="执行详情" name="details">
        <pre class="terminal-log">{{ diagnostics }}</pre>
      </el-collapse-item>
    </el-collapse>
    <el-alert
      v-if="transactionNeedsManual || result?.recoveryFailed"
      title="请联系工程师完成人工恢复"
      type="error"
      :closable="false"
      show-icon
    >
      <p>请提供事务编号、快照路径和执行详情，保留全部备份，不要清空数据或再次部署。</p>
      <ol>
        <li>确认目标服务均已停止，核对文件与数据卷备份的完整性。</li>
        <li>按快照清单恢复部署文件，再按数据卷清单恢复部署前的数据。</li>
        <li>使用上一版本的配置与镜像启动服务，检查健康状态及网页访问后再恢复业务。</li>
      </ol>
    </el-alert>
    <div class="step-footer">
      <el-button :disabled="busy" @click="emit('previous')"> 返回清单比对 </el-button>
      <div class="input-row wrap">
        <el-button
          v-if="transactionBlocked && !transactionNeedsManual"
          type="warning"
          :loading="recoveringTransaction"
          :disabled="busy"
          @click="emit('recover')"
        >
          恢复上一版本
        </el-button>
        <template v-else>
          <el-button :loading="preflighting" :disabled="!canExecute || locked" @click="emit('preflight')">
            执行预检
          </el-button>
          <el-button
            type="primary"
            :loading="executing"
            :disabled="!canExecute || locked"
            @click="emit('execute')"
          >
            开始完整版本部署
          </el-button>
        </template>
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
  plan: { type: Object, default: null },
  review: { type: Object, default: null },
  transaction: { type: Object, default: null },
  busy: { type: Boolean, default: false },
  canExecute: { type: Boolean, default: false },
  artifactSelectionIssue: { type: String, default: '' },
  preflighting: { type: Boolean, default: false },
  preflightPassed: { type: Boolean, default: false },
  preflightMessage: { type: String, default: '' },
  executing: { type: Boolean, default: false },
  recoveringTransaction: { type: Boolean, default: false },
  phase: { type: String, default: '' },
  operationError: { type: String, default: '' },
  progressPercent: { type: Number, default: 0 },
  progressStep: { type: String, default: '' },
  progressMessage: { type: String, default: '' },
  elapsedText: { type: String, default: '' },
  result: { type: Object, default: null },
  diagnostics: { type: String, default: '' }
});

const emit = defineEmits(['previous', 'preflight', 'execute', 'go-dashboard', 'recover']);

const services = computed(() =>
  (props.plan?.services?.length
    ? props.plan.services
    : props.review?.services || []
  ).filter(
    (service) => service.change !== 'removed'
  ).map((service) => ({
    ...service,
    key: service.key || service.name,
    label: service.label || service.name || service.key
  }))
);
const dataVolumes = computed(() => props.plan?.dataVolumes || []);
const dataPaths = computed(() => props.plan?.dataPaths || []);
const transactionBlocked = computed(
  () =>
    Boolean(props.transaction) &&
    !['committed', 'recovered'].includes(props.transaction.state)
);
const transactionNeedsManual = computed(
  () => ['recovery_failed', 'recovering'].includes(props.transaction?.state)
);
const backupEstimate = computed(() => {
  const match = props.preflightMessage.match(/预计备份空间约\s*([\d.]+\s*GB)/);
  return match ? match[1] : '预检时按数据量估算';
});
const locked = computed(() => props.busy || Boolean(props.result?.success));
const resultTitle = computed(() => {
  if (props.result?.success) return '完整版本部署完成';
  if (props.result?.recovered) return '新版本部署失败，已恢复上一版本';
  if (props.result?.recoveryFailed) return '新版本失败，自动恢复未完成';
  return '部署未完成';
});
const resultTone = computed(() => {
  if (props.result?.success) return 'success';
  if (props.result?.recovered) return 'warning';
  return 'error';
});
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
.deployment-summary {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: 12px;
  margin: 18px 0;
}
.summary-item {
  padding: 14px 16px;
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
}
.summary-label {
  display: block;
  margin-bottom: 5px;
  color: var(--text-muted);
  font-size: 12px;
}
.summary-item strong {
  font-size: 13px;
  color: var(--text-main);
}
.deployment-detail {
  margin: 10px 0 18px;
}
.tag-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.field-hint {
  width: 100%;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.65;
  margin-top: 6px;
  overflow-wrap: anywhere;
}
.spaced {
  margin-top: 14px;
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
.optional-panel,
.execution-result,
.execution-status {
  margin-top: 20px;
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
@media (max-width: 900px) {
  .deployment-summary {
    grid-template-columns: 1fr 1fr;
  }
}
@media (max-width: 760px) {
  .step-card {
    padding: 20px;
  }
  .deployment-summary {
    grid-template-columns: 1fr;
  }
  .step-footer {
    flex-wrap: wrap;
  }
}
</style>
