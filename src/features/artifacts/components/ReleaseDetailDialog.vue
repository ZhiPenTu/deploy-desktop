<template>
  <el-dialog
    :model-value="visible"
    :title="title"
    width="880px"
    @update:model-value="onVisibleChange"
    @closed="emit('closed')"
  >
    <div v-if="release" class="release-body detail-body">
      <div class="release-grid">
        <span class="label">版本</span>
        <span class="value">{{ versionText(release) }}</span>
        <template v-if="release.runId">
          <span class="label">运行编号</span>
          <span class="value">#{{ release.runId }}</span>
          <span class="label">提交</span>
          <span class="value">{{ release.commitShort || '-' }} {{ release.commitTitle }}</span>
          <span class="label">分支</span>
          <span class="value">{{ release.branch || '-' }}</span>
        </template>
        <span class="label">构建时间</span>
        <span class="value">{{ artifactTimeText(release.builtAt) }}</span>
        <span class="label">本地版本</span>
        <span class="value">
          {{ localStatusText(release) }}
          <span v-if="release.localDir" class="detail-path">{{ shortPath(release.localDir) }}</span>
        </span>
      </div>

      <p v-if="!release.localDir" class="detail-hint">
        该版本尚未下载到本机。下载并解包后即可查看交付包链路与部署内容。
      </p>
      <div v-else-if="loading" v-loading="true" class="plan-loading card-surface">
        <span>正在读取交付包内容</span>
      </div>
      <el-alert v-else-if="error" :title="error" type="warning" :closable="false" show-icon />
      <BundlePlanPanel v-else-if="plan" :plan="plan" :target="target" />
    </div>

    <template #footer>
      <el-button v-if="release && release.runUrl" link type="primary" @click="emit('open-run', release)">
        打开运行详情
      </el-button>
      <el-button @click="close">关闭</el-button>
      <el-button
        v-if="release && release.localDir"
        type="primary"
        :loading="deploying"
        @click="emit('deploy', release)"
      >
        部署
      </el-button>
      <el-button
        v-else-if="release && !release.localDir && release.downloadUrl"
        type="primary"
        @click="emit('download', release)"
      >
        下载并解包
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed } from 'vue';
import BundlePlanPanel from './BundlePlanPanel.vue';
import { artifactTimeText, localStatusText, shortPath, versionText } from '../domain/releases.js';

const props = defineProps({
  visible: { type: Boolean, default: false },
  release: { type: Object, default: null },
  plan: { type: Object, default: null },
  loading: { type: Boolean, default: false },
  error: { type: String, default: '' },
  target: { type: Object, default: null },
  deploying: { type: Boolean, default: false }
});

const emit = defineEmits(['update:visible', 'closed', 'open-run', 'deploy', 'download']);

const title = computed(() =>
  props.release ? `版本详情 ${versionText(props.release)}` : '版本详情'
);

function onVisibleChange(value) {
  if (value) return;
  emit('update:visible', false);
}

function close() {
  emit('update:visible', false);
}
</script>

<style scoped>
.release-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.release-grid {
  display: grid;
  grid-template-columns: 92px 1fr;
  row-gap: 10px;
  column-gap: 12px;
  font-size: 13px;
}

.release-grid .label {
  color: var(--text-muted);
}

.release-grid .value {
  color: var(--text-main);
  word-break: break-all;
}

.plan-loading {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 140px;
  font-size: 13px;
  color: var(--text-muted);
}

.detail-body {
  max-height: 64vh;
  overflow-y: auto;
  padding-right: 4px;
}

.detail-hint {
  margin: 0;
  padding: 14px 16px;
  border-radius: var(--radius-md);
  background: var(--panel-subtle);
  color: var(--text-muted);
  font-size: 12.5px;
  line-height: 1.6;
}

.detail-path {
  margin-left: 8px;
  color: var(--text-muted);
  font-size: 12px;
}
</style>
