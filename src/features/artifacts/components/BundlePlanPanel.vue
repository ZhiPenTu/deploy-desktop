<template>
  <div class="plan-panel card-surface">
    <div class="plan-head">
      <div class="plan-heading">
        <h3>交付包链路</h3>
        <div class="plan-meta">
          <el-tag size="small" type="success" effect="plain">{{ runtimeLabel }}</el-tag>
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

    <div class="plan-block">
      <div class="block-head">
        <h4>部署内容</h4>
        <span class="block-count">{{ artifactCount }} 个产物 · {{ plan.fileCount }} 个文件 · {{ formatSize(plan.totalBytes) }}</span>
      </div>
      <div v-if="!components.length" class="block-empty">交付包未声明可更新的组件</div>
      <div v-else class="component-list">
        <section v-for="component in components" :key="component.key" class="component-group">
          <div class="component-head">
            <span class="component-name">{{ component.label }}</span>
            <el-tag
              v-for="service in component.services"
              :key="service"
              size="small"
              effect="plain"
              type="info"
            >{{ serviceLabel(service) }}</el-tag>
          </div>
          <ul v-if="component.artifacts.length" class="artifact-list">
            <li v-for="artifact in component.artifacts" :key="artifact.key">
              <div class="artifact-flow">
                <code>{{ artifact.source }}</code>
                <span class="flow-arrow">写入</span>
                <code>{{ targetLabel(artifact) }}</code>
              </div>
              <span class="artifact-meta">{{ kindLabel(artifact.kind) }} · {{ formatSize(artifact.sizeBytes) }}</span>
            </li>
          </ul>
          <div v-else class="block-empty">该组件只更新容器配置</div>
        </section>
      </div>
    </div>

    <div class="plan-block">
      <div class="block-head">
        <h4>运行服务</h4>
        <span class="block-count">{{ plan.services.length }} 个服务</span>
      </div>
      <div v-if="!plan.services.length" class="block-empty">交付包未声明运行服务</div>
      <ul v-else class="service-list">
        <li v-for="service in plan.services" :key="service.key">
          <code>{{ service.container }}</code>
          <span class="service-meta">
            {{ service.label }}
            <template v-if="service.port"> · {{ service.port }}/{{ service.protocol }}</template>
          </span>
        </li>
      </ul>
      <div v-if="plan.database" class="block-tip">
        数据库 {{ plan.database.name }}（{{ plan.database.client }}，端口 {{ plan.database.port }}）
      </div>
    </div>

    <div class="plan-block images-block">
      <div class="block-head">
        <h4>镜像准备</h4>
        <span class="block-count">{{ imageCheckLabel }}</span>
      </div>
      <p class="block-tip">
        随包镜像会在部署时导入目标机；标注为基础镜像的条目由 Compose 清单引用，首次部署前准备一次即可。
      </p>
      <div v-if="!plan.images.length" class="block-empty">交付包未声明镜像需求</div>
      <ul v-else class="image-list">
        <li v-for="image in plan.images" :key="image.imageRef">
          <code>{{ image.imageRef }}</code>
          <span class="image-source">{{ roleLabel(image) }}</span>
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
      :file="plan.compose"
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

const configVisible = ref(false);

const components = computed(() =>
  (Array.isArray(props.plan.components) ? props.plan.components : [])
    .map((component) => ({
      ...component,
      artifacts: Array.isArray(component.artifacts) ? component.artifacts : []
    }))
);

const artifactCount = computed(() =>
  components.value.reduce((total, component) => total + component.artifacts.length, 0)
);

const runtimeLabel = computed(() => {
  if (props.plan.runtime === 'compose') return 'Compose 部署';
  return props.plan.runtime || '运行时未声明';
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

function stateLabel(state) {
  if (state === 'ready') return '就绪';
  if (state === 'optional') return '按需';
  if (state === 'blocked') return '待补齐';
  return state;
}

function rootLabel(root) {
  return root === 'install' ? '安装目录' : '部署目录';
}

function targetLabel(artifact) {
  return `${rootLabel(artifact.root)}/${artifact.target}`;
}

function kindLabel(kind) {
  return kind === 'dir' ? '目录' : '文件';
}

function roleLabel(image) {
  const role = image.role === 'base' ? '基础镜像' : '应用镜像';
  return image.required ? `${role} · 必需` : role;
}

function serviceLabel(key) {
  const service = (props.plan.services || []).find((item) => item.key === key);
  return service?.label || key;
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

.service-list,
.artifact-list,
.image-list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.service-list li,
.image-list li {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  color: var(--text-secondary);
  min-width: 0;
}

.service-list li code,
.image-list li code {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11.5px;
  color: var(--text-main);
}

.service-meta {
  flex: none;
  font-size: 11.5px;
  color: var(--text-faint);
}

.component-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.component-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-bg);
}

.component-head {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.component-name {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-main);
}

.artifact-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.artifact-list li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-width: 0;
}

.artifact-flow {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: 11.5px;
}

.artifact-flow code {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-main);
}

.flow-arrow {
  flex: none;
  font-size: 11px;
  color: var(--text-faint);
}

.artifact-meta {
  flex: none;
  font-size: 11px;
  color: var(--text-faint);
}

.image-source {
  flex: none;
  font-size: 11px;
  color: var(--text-faint);
}

</style>
