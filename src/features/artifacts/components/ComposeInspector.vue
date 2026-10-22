<template>
  <el-dialog
    :model-value="modelValue"
    :title="dialogTitle"
    width="min(1080px, calc(100vw - 32px))"
    top="6vh"
    append-to-body
    destroy-on-close
    class="compose-dialog"
    @update:model-value="emit('update:modelValue', $event)"
  >
    <div v-loading="loading" class="compose-body">
      <div v-if="error" class="read-error">
        <el-alert :title="error" type="error" :closable="false" show-icon />
        <el-button :loading="loading" @click="load">重新检查</el-button>
      </div>

      <template v-else-if="view">
        <div class="meta-row">
          <code class="path">{{ view.file }}</code>
          <el-tag size="small" effect="plain">{{ kindLabel }}</el-tag>
          <span class="meta-text">{{ composeSizeText(view.sizeBytes) }}</span>
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
            :aria-pressed="filter === card.key"
            @click="toggleFilter(card.key)"
          >
            <span class="summary-num">{{ card.count }}</span>
            <span class="summary-label">{{ card.label }}</span>
          </button>
          <div class="summary-note">
            <span>离线包目录 <code>{{ view.imageDir }}</code></span>
            <span v-if="!view.imageDirExists" class="note-warn">目录不存在</span>
            <div class="directory-actions">
              <el-button size="small" :loading="selectingDirectory" :disabled="loading" @click="chooseImageDir">
                选择目录
              </el-button>
              <el-button size="small" :loading="loading" :disabled="selectingDirectory" @click="load">
                重新检查
              </el-button>
            </div>
          </div>
        </div>

        <el-tabs v-if="view.kind === 'compose'" v-model="tab" class="compose-tabs">
          <el-tab-pane label="镜像对比" name="images">
            <ComposeImageTable
              :images="filteredImages"
              :filter="filter"
              :image-check="view.imageCheck"
              :issues="view.issues"
            />
          </el-tab-pane>

          <el-tab-pane label="服务清单" name="services">
            <ComposeServiceTable :services="view.services" />
          </el-tab-pane>

          <el-tab-pane label="配置内容" name="content">
            <ComposeContentView :content="view.content" :file="view.file" :max-lines="MAX_LINES" />
          </el-tab-pane>
        </el-tabs>

        <ComposeContentView
          v-else
          :content="view.content"
          :file="view.file"
          :max-lines="MAX_LINES"
          standalone
          :show-truncate="false"
        />

        <p class="body-hint">{{ footerHint }}</p>
      </template>
    </div>

    <template #footer>
      <el-button @click="emit('update:modelValue', false)">关闭</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { inspectBundleConfig, selectLocalPath } from '../../../api/tauri.js';
import { createRequestScope } from '../../../shared/async/requestScope.js';
import {
  composeFooterHint,
  composeKindLabel,
  composeSizeText,
  imageAvailableLabel,
  summaryCards as composeSummaryCards
} from '../domain/compose.js';
import ComposeContentView from './ComposeContentView.vue';
import ComposeImageTable from './ComposeImageTable.vue';
import ComposeServiceTable from './ComposeServiceTable.vue';

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
const selectedImageDir = ref('');
const selectingDirectory = ref(false);
const requests = createRequestScope();

const dialogTitle = computed(() => {
  const name = String(props.file || '').split('/').filter(Boolean).pop() || '配置文件';
  return `部署配置 · ${name}`;
});

const kindLabel = computed(() => composeKindLabel(view.value?.kind));

const summaryCards = computed(() =>
  composeSummaryCards(view.value?.images || [], imageAvailableLabel(view.value?.imageCheck))
);

const filteredImages = computed(() => {
  const images = view.value?.images || [];
  if (!filter.value) return images;
  return images.filter((image) => image.status === filter.value);
});

const footerHint = computed(() => composeFooterHint(view.value));

function toggleFilter(key) {
  filter.value = filter.value === key ? '' : key;
}

async function load() {
  const ticket = requests.begin();
  if (!props.modelValue) return;
  const bundleDir = String(props.bundleDir || '').trim();
  const file = String(props.file || '').trim();
  if (!bundleDir || !file) return;

  loading.value = true;
  error.value = '';
  try {
    const result = await inspectBundleConfig(bundleDir, file, props.target, selectedImageDir.value || props.imageDir);
    if (!requests.isCurrent(ticket)) return;
    view.value = result;
    if (view.value.kind !== 'compose') {
      tab.value = 'content';
    }
  } catch {
    if (!requests.isCurrent(ticket)) return;
    error.value = '无法读取该部署配置，请确认交付包内容完整后重试。';
  } finally {
    if (requests.isCurrent(ticket)) loading.value = false;
  }
}

async function chooseImageDir() {
  const ticket = requests.begin();
  selectingDirectory.value = true;
  try {
    const selected = await selectLocalPath({ directory: true, multiple: false, title: '选择离线包目录' });
    if (!requests.isCurrent(ticket) || !selected) return;
    selectedImageDir.value = selected;
    filter.value = '';
    await load();
  } catch {
    if (requests.isCurrent(ticket)) ElMessage.error('无法选择目录，请重试。');
  } finally {
    selectingDirectory.value = false;
  }
}

watch(
  () => [props.modelValue, props.file, props.bundleDir, props.imageDir, props.target?.host, props.target?.port, props.target?.user, props.target?.keyPath],
  () => {
    selectedImageDir.value = '';
    view.value = null;
    tab.value = 'images';
    filter.value = '';
    load();
  },
  { immediate: true }
);

onBeforeUnmount(() => requests.dispose());
</script>

<style scoped>
.compose-body {
  min-height: 280px;
}

.read-error {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 12px;
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
  flex: 1 1 280px;
  min-width: 0;
  font-size: 11.5px;
  color: var(--text-faint);
  text-align: right;
}

.summary-note code {
  font-size: 11px;
  color: var(--text-secondary);
  overflow-wrap: anywhere;
}

.directory-actions {
  display: flex;
  justify-content: flex-end;
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

.body-hint {
  margin-top: 10px;
  font-size: 11.5px;
  line-height: 1.6;
  color: var(--text-faint);
}
</style>
