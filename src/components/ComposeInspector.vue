<template>
  <el-dialog
    :model-value="modelValue"
    :title="dialogTitle"
    width="1080px"
    top="6vh"
    append-to-body
    destroy-on-close
    class="compose-dialog"
    @update:model-value="emit('update:modelValue', $event)"
  >
    <div v-loading="loading" class="compose-body">
      <el-alert v-if="error" :title="error" type="error" :closable="false" show-icon />

      <template v-else-if="view">
        <div class="meta-row">
          <code class="path">{{ view.file }}</code>
          <el-tag size="small" effect="plain">{{ kindLabel }}</el-tag>
          <span class="meta-text">{{ formatSize(view.sizeBytes) }}</span>
          <span v-if="view.services.length" class="meta-text">{{ view.services.length }} 个服务</span>
          <span v-if="view.images.length" class="meta-text">{{ view.images.length }} 个镜像</span>
        </div>

        <div v-if="view.kind === 'compose'" class="compare-summary">
          <button
            v-for="card in summaryCards"
            :key="card.key"
            type="button"
            class="summary-card"
            :class="[card.tone, { active: filter === card.key }]"
            @click="toggleFilter(card.key)"
          >
            <span class="summary-num">{{ card.count }}</span>
            <span class="summary-label">{{ card.label }}</span>
          </button>
          <div class="summary-note">
            <span>镜像包目录 <code>{{ view.imageDir }}</code></span>
            <span v-if="!view.imageDirExists" class="note-warn">目录不存在</span>
            <span v-else-if="view.manifest !== 'none'" class="note-ok">清单来源 {{ view.manifest }}</span>
          </div>
        </div>

        <el-tabs v-if="view.kind === 'compose'" v-model="tab" class="compose-tabs">
          <el-tab-pane label="镜像对比" name="images">
            <el-table
              :data="filteredImages"
              size="small"
              :row-class-name="rowClass"
              empty-text="配置文件中没有 image 声明"
            >
              <el-table-column label="镜像" min-width="320">
                <template #default="{ row }">
                  <div class="image-cell">
                    <code>{{ row.imageRef }}</code>
                    <span v-if="row.matchRef !== row.imageRef" class="image-sub">
                      比对键 {{ row.matchRef }}
                    </span>
                  </div>
                </template>
              </el-table-column>
              <el-table-column label="服务" min-width="160">
                <template #default="{ row }">
                  <el-tag
                    v-for="service in row.services"
                    :key="service"
                    size="small"
                    effect="plain"
                    class="service-tag"
                  >
                    {{ service }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column label="本地镜像包" min-width="240">
                <template #default="{ row }">
                  <div class="image-cell">
                    <code v-if="row.localFile" class="file-name">{{ row.localFile }}</code>
                    <span v-else class="missing-text">未找到镜像包</span>
                    <span v-if="row.localFile" class="image-sub">
                      {{ formatSize(row.localBytes) }} · {{ sourceLabel(row.matchSource) }}
                    </span>
                  </div>
                </template>
              </el-table-column>
              <el-table-column :label="dockerColumnLabel" width="130">
                <template #default="{ row }">
                  <el-tag size="small" :type="dockerTone(row)" effect="plain">
                    {{ dockerLabel(row) }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column label="状态" width="110">
                <template #default="{ row }">
                  <el-tag size="small" :type="statusTone(row.status)" effect="light">
                    {{ statusLabel(row.status) }}
                  </el-tag>
                </template>
              </el-table-column>
            </el-table>

            <ul v-if="view.issues.length" class="issue-list">
              <li v-for="issue in view.issues" :key="issue">{{ issue }}</li>
            </ul>
          </el-tab-pane>

          <el-tab-pane label="服务清单" name="services">
            <el-table :data="view.services" size="small" empty-text="配置文件中没有 services">
              <el-table-column prop="name" label="服务" width="190" />
              <el-table-column label="镜像" min-width="280">
                <template #default="{ row }">
                  <code>{{ row.image || '-' }}</code>
                </template>
              </el-table-column>
              <el-table-column prop="containerName" label="容器名" min-width="200" />
              <el-table-column label="端口" min-width="130">
                <template #default="{ row }">
                  <span v-if="!row.ports.length" class="muted">-</span>
                  <el-tag v-for="port in row.ports" :key="port" size="small" effect="plain" class="service-tag">
                    {{ port }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column label="依赖" min-width="150">
                <template #default="{ row }">
                  <span v-if="!row.dependsOn.length" class="muted">-</span>
                  <el-tag
                    v-for="dependency in row.dependsOn"
                    :key="dependency"
                    size="small"
                    effect="plain"
                    class="service-tag"
                  >
                    {{ dependency }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column label="重启策略" width="130">
                <template #default="{ row }">
                  <span class="muted">{{ row.restart || '-' }}</span>
                </template>
              </el-table-column>
            </el-table>
          </el-tab-pane>

          <el-tab-pane label="配置内容" name="content">
            <div class="code-view">
              <div
                v-for="line in lines"
                :key="line.no"
                class="code-line"
                :class="{ highlight: line.highlight }"
              >
                <span class="line-no">{{ line.no }}</span>
                <span class="line-text">{{ line.text }}</span>
              </div>
            </div>
            <p v-if="truncatedLines" class="truncate-hint">
              内容较长，仅渲染前 {{ maxLines }} 行，完整文件请直接打开 {{ view.file }}
            </p>
          </el-tab-pane>
        </el-tabs>

        <div v-else class="code-view single">
          <div
            v-for="line in lines"
            :key="line.no"
            class="code-line"
            :class="{ highlight: line.highlight }"
          >
            <span class="line-no">{{ line.no }}</span>
            <span class="line-text">{{ line.text }}</span>
          </div>
        </div>

        <p class="body-hint">{{ footerHint }}</p>
      </template>
    </div>

    <template #footer>
      <el-button @click="emit('update:modelValue', false)">关闭</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { inspectBundleConfig } from '../api/tauri.js';

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  bundleDir: { type: String, default: '' },
  file: { type: String, default: '' },
  target: { type: Object, default: null },
  imageDir: { type: String, default: '' }
});

const emit = defineEmits(['update:modelValue']);

const MAX_LINES = 1500;

const loading = ref(false);
const error = ref('');
const view = ref(null);
const tab = ref('images');
const filter = ref('');

const dialogTitle = computed(() => {
  const name = String(props.file || '').split('/').filter(Boolean).pop() || '配置文件';
  return `部署配置 · ${name}`;
});

const kindLabel = computed(() => {
  if (!view.value) return '';
  if (view.value.kind === 'compose') return 'compose 清单';
  if (view.value.kind === 'yaml') return 'YAML';
  return '文本';
});

const lines = computed(() => {
  const content = view.value?.content || '';
  const raw = content.split('\n');
  return raw.slice(0, MAX_LINES).map((text, index) => ({
    no: index + 1,
    text: text || ' ',
    highlight: /^\s*(-\s*)?image:\s*\S/.test(text)
  }));
});

const truncatedLines = computed(() => {
  const content = view.value?.content || '';
  return content.split('\n').length > MAX_LINES;
});

const summaryCards = computed(() => {
  const images = view.value?.images || [];
  const count = (status) => images.filter((image) => image.status === status).length;
  return [
    { key: 'ready', label: '就绪', count: count('ready'), tone: 'ok' },
    { key: 'to_import', label: '待导入', count: count('to_import'), tone: 'warn' },
    { key: 'missing', label: '缺失', count: count('missing'), tone: 'danger' },
    { key: 'unconfirmed', label: '未确认', count: count('unconfirmed'), tone: 'unknown' },
    { key: 'loaded', label: '仅本机已有', count: count('loaded'), tone: 'info' }
  ];
});

const filteredImages = computed(() => {
  const images = view.value?.images || [];
  if (!filter.value) return images;
  return images.filter((image) => image.status === filter.value);
});

const dockerColumnLabel = computed(() => {
  if (view.value?.imageCheck === 'remote') return '目标机 Docker';
  if (view.value?.imageCheck === 'unavailable') return 'Docker 状态';
  return '本机 Docker';
});

const footerHint = computed(() => {
  if (!view.value) return '';
  if (view.value.kind !== 'compose') return '该文件仅展示内容，镜像对比只针对含 services 的 compose 文件';
  if (!view.value.images.length) return '配置文件中没有 image 声明';
  return '镜像清单由 compose 的 image 字段推导，镜像包按交付包清单、部署配置映射、文件名依次匹配';
});

function formatSize(bytes) {
  const size = Number(bytes || 0);
  if (!size) return '0 B';
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
  return `${(size / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function sourceLabel(source) {
  if (source === 'images.json') return '交付包清单';
  if (source === 'profile') return '部署配置映射';
  if (source === 'filename') return '按文件名推断';
  return '来源未知';
}

function statusLabel(status) {
  if (status === 'ready') return '就绪';
  if (status === 'to_import') return '待导入';
  if (status === 'loaded') return '仅本机已有';
  if (status === 'missing') return '缺失';
  if (status === 'unconfirmed') return '未确认';
  return status;
}

function statusTone(status) {
  if (status === 'ready') return 'success';
  if (status === 'to_import') return 'warning';
  if (status === 'missing') return 'danger';
  if (status === 'unconfirmed') return 'info';
  return 'info';
}

function dockerTone(row) {
  if (row.dockerPresent) return 'success';
  return view.value?.imageCheck === 'unavailable' ? 'info' : 'warning';
}

function dockerLabel(row) {
  if (row.dockerPresent) return '已加载';
  return view.value?.imageCheck === 'unavailable' ? '未确认' : '未加载';
}

function rowClass({ row }) {
  return `status-${row.status}`;
}

function toggleFilter(key) {
  filter.value = filter.value === key ? '' : key;
}

async function load() {
  if (!props.modelValue) return;
  const bundleDir = String(props.bundleDir || '').trim();
  const file = String(props.file || '').trim();
  if (!bundleDir || !file) return;

  loading.value = true;
  error.value = '';
  view.value = null;
  tab.value = 'images';
  filter.value = '';
  try {
    view.value = await inspectBundleConfig(bundleDir, file, props.target, props.imageDir);
    if (view.value.kind !== 'compose') {
      tab.value = 'content';
    }
  } catch (caught) {
    error.value = `读取配置失败：${caught}`;
  } finally {
    loading.value = false;
  }
}

watch(
  () => [props.modelValue, props.file, props.bundleDir],
  () => load()
);
</script>

<style scoped>
.compose-body {
  min-height: 280px;
}

.meta-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  margin-bottom: 12px;
}

.meta-row .path {
  font-size: 12.5px;
  color: var(--text-main);
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  padding: 3px 8px;
}

.meta-text {
  font-size: 12px;
  color: var(--text-muted);
}

.compare-summary {
  display: flex;
  align-items: stretch;
  gap: 10px;
  flex-wrap: wrap;
  padding: 12px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--panel-subtle);
  margin-bottom: 12px;
}

.summary-card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  min-width: 88px;
  padding: 8px 12px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-color);
  background: #ffffff;
  cursor: pointer;
  text-align: left;
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}

.summary-card:hover {
  border-color: var(--border-strong);
}

.summary-card.active {
  border-color: var(--primary-color);
  box-shadow: 0 0 0 2px var(--primary-ring);
}

.summary-num {
  font-size: 18px;
  font-weight: 600;
  line-height: 1.1;
  color: var(--text-main);
}

.summary-label {
  font-size: 11.5px;
  color: var(--text-muted);
}

.summary-card.ok .summary-num {
  color: var(--success-color);
}

.summary-card.warn .summary-num {
  color: var(--warning-color);
}

.summary-card.danger .summary-num {
  color: var(--danger-color);
}

.summary-card.info .summary-num {
  color: var(--primary-color);
}

.summary-card.unknown .summary-num {
  color: var(--text-muted);
}

.summary-note {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 4px;
  margin-left: auto;
  font-size: 11.5px;
  color: var(--text-faint);
  text-align: right;
}

.summary-note code {
  font-size: 11px;
  color: var(--text-secondary);
}

.note-warn {
  color: var(--warning-color);
}

.note-ok {
  color: var(--success-color);
}

.compose-tabs {
  margin-top: 4px;
}

.image-cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.image-cell code {
  font-size: 12px;
  color: var(--text-main);
  word-break: break-all;
}

.image-sub {
  font-size: 11px;
  color: var(--text-faint);
}

.file-name {
  color: var(--primary-color);
}

.missing-text {
  font-size: 12px;
  color: var(--danger-color);
}

.service-tag {
  margin: 2px 4px 2px 0;
}

.muted {
  color: var(--text-faint);
  font-size: 12px;
}

.issue-list {
  margin: 10px 0 0;
  padding-left: 18px;
  font-size: 11.5px;
  color: var(--warning-color);
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.code-view {
  max-height: 52vh;
  overflow: auto;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: #0b1220;
  padding: 10px 0;
  font-family: 'SFMono-Regular', Menlo, Consolas, monospace;
}

.code-view.single {
  max-height: 62vh;
}

.code-line {
  display: flex;
  gap: 12px;
  padding: 0 12px;
  font-size: 12px;
  line-height: 1.6;
  color: #d7dde8;
  white-space: pre;
}

.code-line.highlight {
  background: rgba(51, 92, 255, 0.16);
  color: #ffffff;
}

.line-no {
  flex: none;
  width: 42px;
  text-align: right;
  color: #5b6b85;
  user-select: none;
}

.line-text {
  flex: 1;
  min-width: 0;
}

.truncate-hint {
  margin-top: 8px;
  font-size: 11.5px;
  color: var(--text-faint);
}

.body-hint {
  margin-top: 10px;
  font-size: 11.5px;
  line-height: 1.6;
  color: var(--text-faint);
}

:deep(.el-table .status-missing td) {
  background: var(--danger-soft);
}

:deep(.el-table .status-to_import td) {
  background: var(--warning-soft);
}
</style>
