<template>
  <div class="plan-panel card-surface">
    <div class="plan-head">
      <div class="plan-heading">
        <h3>交付包链路</h3>
        <div class="plan-meta">
          <el-tag size="small" :type="mode === 'file' ? 'success' : 'primary'" effect="plain">
            {{ modeLabel }}
          </el-tag>
          <span v-if="plan.version">版本 {{ plan.version }}</span>
          <span v-if="plan.branch">分支 {{ plan.branch }}</span>
          <span v-if="plan.commitShort">提交 {{ plan.commitShort }}</span>
          <span v-if="plan.builtAt">构建 {{ builtAtText }}</span>
        </div>
      </div>
      <div class="plan-actions">
        <slot name="actions" />
      </div>
    </div>

    <div class="chain">
      <div
        v-for="(step, index) in plan.steps"
        :key="step.key"
        class="chain-node"
        :class="step.state"
      >
        <div class="node-top">
          <span class="node-index">{{ index + 1 }}</span>
          <span class="node-title">{{ step.title }}</span>
          <span class="node-state">{{ stateLabel(step.state) }}</span>
        </div>
        <span class="node-detail">{{ step.detail }}</span>
        <span v-if="step.hint" class="node-hint">{{ step.hint }}</span>
      </div>
    </div>

    <div class="plan-blocks">
      <div class="plan-block">
        <div class="block-head">
          <h4>应用产物 app/</h4>
          <span class="block-count">{{ plan.appFiles.length }} 个文件</span>
        </div>
        <div v-if="!plan.appFiles.length" class="block-empty">
          交付包内没有 app/ 目录，请确认制品类型
        </div>
        <ul v-else class="file-list">
          <li v-for="file in visibleAppFiles" :key="file.path">
            <code>{{ file.path }}</code>
            <span>{{ formatSize(file.sizeBytes) }}</span>
          </li>
          <li v-if="plan.appFiles.length > limit && !expanded" class="more-row">
            <el-button link type="primary" @click="expanded = true">
              展开其余 {{ plan.appFiles.length - limit }} 个文件
            </el-button>
          </li>
        </ul>
      </div>

      <div class="plan-block">
        <div class="block-head">
          <h4>部署配置 deploy/</h4>
          <span class="block-count">{{ plan.configFiles.length }} 个文件</span>
        </div>
        <div v-if="!plan.configFiles.length" class="block-empty">
          交付包内没有 deploy/ 目录，无法启动容器栈
        </div>
        <ul v-else class="file-list">
          <li v-for="file in visibleConfigFiles" :key="file.path" class="config-row">
            <button type="button" class="file-button" @click="openConfig(file.path)">
              <el-icon><Document /></el-icon>
              <code>{{ file.path }}</code>
            </button>
            <el-tag
              v-if="isComposeFile(file.path)"
              size="small"
              type="success"
              effect="plain"
              class="file-badge"
            >
              清单
            </el-tag>
            <span>{{ formatSize(file.sizeBytes) }}</span>
          </li>
          <li v-if="plan.configFiles.length > limit && !expanded" class="more-row">
            <el-button link type="primary" @click="expanded = true">
              展开其余 {{ plan.configFiles.length - limit }} 个文件
            </el-button>
          </li>
        </ul>
      </div>
    </div>

    <div class="plan-block images-block">
      <div class="block-head">
        <h4>{{ mode === 'file' ? '依赖基础镜像' : '随包镜像' }}</h4>
        <span class="block-count">{{ imageCheckLabel }}</span>
      </div>
      <p class="block-tip">
        <template v-if="mode === 'file'">
          文件模式交付包只含 app/ 与 deploy/，容器直接挂载这些产物。
          这里的镜像不随包分发，是目标机运行容器所需的基础镜像，首次部署时准备一次即可。
        </template>
        <template v-else>
          镜像模式交付包自带 images/ 镜像包，必须先导入目标机，容器才能用新镜像重建。
        </template>
      </p>
      <div v-if="!plan.images.length" class="block-empty">交付包未声明镜像需求</div>
      <ul v-else class="image-list">
        <li v-for="image in plan.images" :key="image.imageRef">
          <code>{{ image.imageRef }}</code>
          <span class="image-source">{{ sourceLabel(image.source) }}</span>
          <el-tag
            size="small"
            :type="image.presentLocally ? 'success' : (imageCheck === 'unavailable' ? 'info' : 'warning')"
            effect="plain"
          >
            {{ presenceLabel(image.presentLocally) }}
          </el-tag>
        </li>
      </ul>
    </div>

    <ComposeInspector
      v-model="configVisible"
      :bundle-dir="plan.bundleDir"
      :file="configFile"
      :target="target"
      :image-dir="imageDir"
    />
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import ComposeInspector from './ComposeInspector.vue';

const props = defineProps({
  plan: {
    type: Object,
    required: true
  },
  limit: {
    type: Number,
    default: 6
  },
  target: {
    type: Object,
    default: null
  },
  imageDir: {
    type: String,
    default: ''
  }
});

const expanded = ref(false);
const configVisible = ref(false);
const configFile = ref('');

function isComposeFile(path) {
  return /(^|\/)docker-compose[^/]*\.ya?ml$/i.test(String(path || ''));
}

function openConfig(path) {
  configFile.value = String(path || '');
  configVisible.value = true;
}

const mode = computed(() => props.plan.mode || 'unknown');

const modeLabel = computed(() => {
  if (mode.value === 'file') return '文件模式';
  if (mode.value === 'image') return '镜像模式';
  return '模式未识别';
});

const builtAtText = computed(() => {
  const raw = props.plan.builtAt;
  if (!raw) return '';
  const date = new Date(raw);
  if (Number.isNaN(date.getTime())) return raw;
  return date.toLocaleString();
});

const imageCheck = computed(() => props.plan.imageCheck || 'local');

const imageCheckLabel = computed(() => {
  if (props.plan.imageCheck === 'remote') return '已检查目标机';
  if (props.plan.imageCheck === 'unavailable') return '目标机镜像状态未确认';
  return '已检查本机';
});

const visibleAppFiles = computed(() =>
  expanded.value ? props.plan.appFiles : props.plan.appFiles.slice(0, props.limit)
);

const visibleConfigFiles = computed(() =>
  expanded.value ? props.plan.configFiles : props.plan.configFiles.slice(0, props.limit)
);

function stateLabel(state) {
  if (state === 'ready') return '就绪';
  if (state === 'optional') return '按需';
  if (state === 'blocked') return '待补齐';
  return state;
}

function sourceLabel(source) {
  if (source === 'bundle') return '随包镜像';
  if (source === 'declared') return '基础镜像清单';
  if (source === 'compose') return 'compose 推导';
  return source;
}

function presenceLabel(present) {
  if (props.plan.imageCheck === 'unavailable') return '未确认';
  return present ? '已具备' : '缺失';
}

function formatSize(bytes) {
  const value = Number(bytes || 0);
  if (!value) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}
</script>

<style scoped>
.plan-panel {
  padding: 18px 20px 20px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.plan-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.plan-heading {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.plan-heading h3 {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-main);
}

.plan-meta {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  font-size: 12px;
  color: var(--text-muted);
}

.plan-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.chain {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 10px;
}

.chain-node {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 12px;
  border-radius: var(--radius-md);
  border: 1px solid var(--border-color);
  background: var(--panel-subtle);
  min-width: 0;
}

.chain-node .node-top {
  display: flex;
  align-items: flex-start;
  gap: 8px;
}

.node-index {
  width: 20px;
  height: 20px;
  margin-top: 1px;
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  font-size: 11px;
  font-weight: 600;
  background: var(--border-color);
  color: var(--text-secondary);
}

.node-title {
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  line-height: 1.4;
  font-weight: 600;
  color: var(--text-main);
}

.node-state {
  flex: none;
  font-size: 11px;
  line-height: 1.4;
  color: var(--text-muted);
}

.node-detail {
  font-size: 11.5px;
  line-height: 1.5;
  color: var(--text-muted);
  word-break: break-word;
}

.node-hint {
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-faint);
  word-break: break-word;
}

.chain-node.ready {
  border-color: rgba(14, 159, 110, 0.28);
  background: var(--success-soft);
}

.chain-node.ready .node-index {
  background: var(--success-color);
  color: #ffffff;
}

.chain-node.ready .node-state {
  color: var(--success-color);
}

.chain-node.optional {
  border-color: var(--border-strong);
  background: var(--panel-subtle);
}

.chain-node.optional .node-index {
  background: var(--primary-color);
  color: #ffffff;
}

.chain-node.optional .node-state {
  color: var(--primary-color);
}

.chain-node.blocked {
  border-color: rgba(217, 45, 32, 0.28);
  background: var(--danger-soft);
}

.chain-node.blocked .node-index {
  background: var(--danger-color);
  color: #ffffff;
}

.chain-node.blocked .node-state {
  color: var(--danger-color);
}

.plan-blocks {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px;
}

.plan-block {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 14px;
  border-radius: var(--radius-md);
  border: 1px solid var(--border-color);
  background: var(--panel-subtle);
}

.block-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.block-head h4 {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
}

.block-count {
  font-size: 11.5px;
  color: var(--text-muted);
}

.block-tip {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-muted);
}

.block-empty {
  font-size: 12px;
  color: var(--text-faint);
}

.file-list,
.image-list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.file-list li,
.image-list li {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  color: var(--text-secondary);
  min-width: 0;
}

.file-list li code,
.image-list li code {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11.5px;
  color: var(--text-main);
  background: transparent;
}

.file-list li > span {
  flex: none;
  color: var(--text-faint);
  font-size: 11.5px;
}

.image-source {
  flex: none;
  font-size: 11px;
  color: var(--text-faint);
}

.more-row {
  justify-content: flex-start;
}

.config-row {
  gap: 8px;
}

.file-button {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
  padding: 2px 4px;
  margin-left: -4px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  cursor: pointer;
  text-align: left;
}

.file-button:hover {
  background: var(--primary-soft);
}

.file-button:hover code {
  color: var(--primary-color);
}

.file-button .el-icon {
  flex: none;
  font-size: 12px;
  color: var(--text-faint);
}

.file-button code {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11.5px;
  color: var(--text-main);
  background: transparent;
}

.file-badge {
  flex: none;
}
</style>
