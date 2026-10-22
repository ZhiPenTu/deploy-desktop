<template>
  <el-dialog
    :model-value="visible"
    :title="title"
    width="640px"
    :close-on-click-modal="!downloading"
    @update:model-value="onVisibleChange"
  >
    <div v-if="release" class="release-body">
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
        <span class="label">文件大小</span>
        <span class="value">{{ artifactSizeText(release.sizeBytes) }}</span>
        <span class="label">MD5</span>
        <span class="value"><code>{{ release.md5 || '-' }}</code></span>
      </div>

      <el-progress
        v-if="downloading || progress.percent > 0"
        :percentage="progress.percent"
        :striped="progress.phase === 'extract'"
        :striped-flow="progress.phase === 'extract'"
        :status="progress.percent >= 100 ? 'success' : undefined"
      />
      <p v-if="progress.stage" class="field-hint">{{ progress.stage }}</p>

      <el-alert v-if="downloadError" :title="downloadError" type="error" :closable="false" show-icon>
        <div class="alert-actions">
          <span>下载与校验的详细过程已记录到运行日志。</span>
          <el-button size="small" @click="emit('open-logs')">查看运行日志</el-button>
        </div>
      </el-alert>

      <el-alert
        v-if="bundle"
        :title="`已解包到 ${bundle.dir}`"
        :description="`版本 ${bundle.version || '-'}，共 ${bundle.files} 个文件，交付包校验通过`"
        type="success"
        :closable="false"
        show-icon
      />
    </div>

    <template #footer>
      <el-button v-if="release && release.runUrl" link type="primary" @click="emit('open-run', release)">
        打开运行详情
      </el-button>
      <el-button @click="close">关闭</el-button>
      <el-button v-if="bundle" type="primary" @click="emit('deploy')">部署</el-button>
      <el-button v-else type="primary" :loading="downloading" @click="emit('download')">
        {{ buttonText }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed } from 'vue';
import { artifactSizeText, artifactTimeText, versionText } from '../domain/releases.js';

const props = defineProps({
  visible: { type: Boolean, default: false },
  release: { type: Object, default: null },
  downloading: { type: Boolean, default: false },
  progress: { type: Object, default: () => ({ percent: 0, stage: '', phase: '' }) },
  downloadError: { type: String, default: '' },
  bundle: { type: Object, default: null }
});

const emit = defineEmits(['update:visible', 'download', 'open-run', 'open-logs', 'deploy']);

const title = computed(() =>
  props.release ? `下载版本 ${versionText(props.release)}` : '下载制品版本'
);

const buttonText = computed(() => {
  if (props.downloading) return '正在下载并解包';
  return props.release && props.release.localDir ? '重新下载并解包' : '下载并解包';
});

function onVisibleChange(value) {
  if (value) return;
  close();
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

.alert-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 6px;
}
</style>
