<template>
  <div class="wizard-artifact-picker">
    <el-alert
      v-if="!desktopAvailable"
      title="暂不能读取本机制品"
      description="请在桌面应用中选择、解压并校验交付包。"
      type="info"
      :closable="false"
      show-icon
    />

    <template v-if="stage === 'select'">
      <WizardSourcePickerPanel
        :sources="sources"
        :source-id="sourceId"
        :selected-source="selectedSource"
        :versions="versions"
        :versions-visible="versionsVisible"
        :versions-has-more="versionsHasMore"
        :release="release"
        :candidate="candidate"
        :operation="operation"
        :busy="busy"
        :controls-disabled="controlsDisabled"
        :desktop-available="desktopAvailable"
        @update:source-id="(value) => (sourceId = value)"
        @refresh="refreshSources"
        @go-artifacts="emit('go-artifacts')"
        @choose-downloaded="chooseDownloaded"
        @check-latest="checkLatest"
        @toggle-versions="toggleVersions"
        @load-more="loadVersions(false)"
        @choose-version="chooseVersion"
        @choose-release="chooseRelease"
      />

      <WizardLocalPickerPanel
        :current-bundle-dir="connection.bundleDir || ''"
        :controls-disabled="controlsDisabled"
        :desktop-available="desktopAvailable"
        @pick="pickLocal"
        @choose-current="chooseCurrentDirectory"
      />

      <div v-if="candidate" class="selection-summary">
        <el-icon><DocumentChecked /></el-icon>
        <div>
          <strong>已选择：{{ candidate.label }}</strong>
          <p v-if="candidate.version" class="field-hint">版本：{{ versionLabel(candidate.version) }}</p>
          <p v-if="candidate.path" class="path-text">{{ candidate.path }}</p>
          <p class="field-hint">下一步将{{ preparationLabel }}，通过后读取部署清单。</p>
        </div>
      </div>
    </template>

    <WizardPreparePanel
      v-else
      :candidate="candidate"
      :label="preparationLabel"
      :hint="preparationHint"
      :preparing="preparing"
      :controls-disabled="controlsDisabled"
      :desktop-available="desktopAvailable"
      :prepared-bundle="preparedBundle"
      :progress="progress"
      @prepare="prepare"
    />

    <el-alert
      v-if="errorMessage"
      title="制品准备未完成"
      :description="errorMessage"
      type="error"
      :closable="false"
      show-icon
    />
  </div>
</template>

<script setup>
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue';
import { DocumentChecked } from '@element-plus/icons-vue';
import {
  downloadArtifact,
  fetchArtifactRelease,
  isDesktopApp,
  listArtifactReleases,
  listArtifactSources,
  openArtifactBundle,
  openArtifactBundleVersion,
  prepareLocalArtifactBundle,
  selectLocalPath as open
} from '../../../api/tauri.js';
import { applyProjectBundle } from '../../connections/state.js';
import { releaseKey } from '../domain/releases.js';
import { artifactFailureText } from '../domain/errors.js';
import { prepareActionHint, prepareActionLabel, sizeLabel, versionLabel } from '../domain/bundle-selection.js';
import WizardLocalPickerPanel from './WizardLocalPickerPanel.vue';
import WizardPreparePanel from './WizardPreparePanel.vue';
import WizardSourcePickerPanel from './WizardSourcePickerPanel.vue';

const props = defineProps({
  connection: { type: Object, required: true },
  stage: { type: String, default: 'select' },
  disabled: { type: Boolean, default: false },
  preset: { type: Object, default: null }
});

const emit = defineEmits(['chosen', 'selected', 'busy-change', 'go-artifacts']);
const desktopAvailable = isDesktopApp();
const sources = ref([]);
const sourceId = ref('');
const release = ref(null);
const versions = ref([]);
const versionsVisible = ref(false);
const versionsPage = ref(0);
const versionsHasMore = ref(false);
const candidate = ref(null);
const preparedBundle = ref(null);
const operation = ref('');
const errorMessage = ref('');
const progress = reactive({ percent: 0, total: 0, stage: '' });
const initialPreset = props.preset;
const busy = computed(() => Boolean(operation.value));
const preparing = computed(() => operation.value === 'prepare');
const controlsDisabled = computed(() => props.disabled || busy.value);
const selectedSource = computed(() => sources.value.find((source) => source.id === sourceId.value) || null);
const preparationLabel = computed(() => prepareActionLabel(candidate.value));
const preparationHint = computed(() => prepareActionHint(candidate.value));

watch(busy, (value) => emit('busy-change', value), { flush: 'sync' });
watch(sourceId, () => {
  release.value = null;
  versions.value = [];
  versionsVisible.value = false;
  versionsPage.value = 0;
  versionsHasMore.value = false;
  clearSelection();
});

function clearSelection() {
  candidate.value = null;
  preparedBundle.value = null;
  errorMessage.value = '';
  Object.assign(progress, { percent: 0, total: 0, stage: '' });
  emit('chosen', null);
  emit('selected', null);
}

function choose(next) {
  clearSelection();
  candidate.value = next;
  emit('chosen', {
    kind: next.kind,
    label: next.label,
    version: next.version || '',
    path: next.path || ''
  });
}

function applyPreset(preset) {
  const path = String(preset?.dir || '').trim();
  if (!path) return;
  choose({
    kind: 'directory',
    label: preset.label || versionLabel(preset.version),
    version: preset.version || '',
    path
  });
}

async function readSources() {
  sources.value = await listArtifactSources();
  if (!sources.value.some((source) => source.id === sourceId.value)) {
    sourceId.value = sources.value[0]?.id || '';
  }
}

async function refreshSources() {
  if (controlsDisabled.value || !desktopAvailable) return;
  operation.value = 'sources';
  errorMessage.value = '';
  try {
    await readSources();
  } catch (error) {
    errorMessage.value = artifactFailureText(error, '读取制品源');
  } finally {
    operation.value = '';
  }
}

function chooseDownloaded() {
  const source = selectedSource.value;
  if (controlsDisabled.value || !source?.lastBundleDir) return;
  choose({
    kind: 'downloaded',
    sourceId: source.id,
    label: `${source.name}（已下载）`,
    version: source.lastVersion,
    path: source.lastBundleDir
  });
}

async function checkLatest() {
  if (controlsDisabled.value || !selectedSource.value) return;
  operation.value = 'release';
  errorMessage.value = '';
  release.value = null;
  try {
    release.value = await fetchArtifactRelease(selectedSource.value.id);
  } catch (error) {
    errorMessage.value = artifactFailureText(error, '检查最新版本');
  } finally {
    operation.value = '';
  }
}

function chooseRelease() {
  const source = selectedSource.value;
  if (controlsDisabled.value || !source || !release.value) return;
  choose({
    kind: 'release',
    sourceId: source.id,
    label: `${source.name}（${versionLabel(release.value.version)}）`,
    version: release.value.version,
    release: release.value
  });
}

async function toggleVersions() {
  if (controlsDisabled.value || !desktopAvailable || !selectedSource.value) return;
  if (versionsVisible.value) {
    versionsVisible.value = false;
    return;
  }
  versionsVisible.value = true;
  await loadVersions(true);
}

async function loadVersions(reset) {
  const source = selectedSource.value;
  if (!source || operation.value === 'versions') return;
  operation.value = 'versions';
  errorMessage.value = '';
  const nextPage = reset ? 1 : versionsPage.value + 1;
  try {
    const result = await listArtifactReleases(source.id, nextPage);
    if (sourceId.value !== source.id) return;
    if (reset) {
      versions.value = result.items || [];
    } else {
      const known = new Set(versions.value.map(releaseKey));
      const appended = (result.items || []).filter((item) => !known.has(releaseKey(item)));
      versions.value = [...versions.value, ...appended];
    }
    versionsPage.value = result.page || nextPage;
    versionsHasMore.value = Boolean(result.hasMore);
  } catch (error) {
    errorMessage.value = artifactFailureText(error, '读取版本列表');
  } finally {
    operation.value = '';
  }
}

function chooseVersion(item) {
  const source = selectedSource.value;
  if (controlsDisabled.value || !source || !item) return;
  const version = String(item.version || '').trim();
  if (item.localDir && version) {
    choose({
      kind: 'downloaded-version',
      sourceId: source.id,
      label: `${source.name}（${versionLabel(version)}）`,
      version,
      path: item.localDir
    });
    return;
  }
  choose({
    kind: 'release',
    sourceId: source.id,
    label: `${source.name}（${versionLabel(version)}）`,
    version: item.version,
    release: item
  });
}

async function pickLocal(directory) {
  if (controlsDisabled.value || !desktopAvailable) return;
  operation.value = 'pick';
  errorMessage.value = '';
  try {
    const path = await open({
      directory,
      multiple: false,
      title: directory ? '选择已解压的交付包目录' : '选择交付包',
      ...(directory ? {} : { filters: [{ name: '交付包', extensions: ['tgz', 'gz'] }] })
    });
    if (typeof path !== 'string' || !path) return;
    if (!directory && !/\.(tgz|tar\.gz)$/i.test(path)) {
      errorMessage.value = '请选择 .tgz 或 .tar.gz 格式的交付包。';
      return;
    }
    choose({ kind: directory ? 'directory' : 'archive', label: path.split(/[\\/]/).filter(Boolean).at(-1) || path, path });
  } catch (error) {
    errorMessage.value = artifactFailureText(error, directory ? '选择交付包目录' : '选择交付包');
  } finally {
    operation.value = '';
  }
}

function chooseCurrentDirectory() {
  if (controlsDisabled.value || !desktopAvailable) return;
  const path = String(props.connection.bundleDir || '').trim();
  if (path) choose({ kind: 'directory', label: '当前交付包目录', path });
}

function updateProgress(event) {
  if (event.stage === 'download') {
    const total = Number(event.total || 0);
    const downloaded = Number(event.downloaded || 0);
    progress.total = total;
    progress.percent = total > 0 ? Math.min(98, Math.round(downloaded / total * 98)) : 0;
    progress.stage = total > 0 ? `正在下载：${sizeLabel(downloaded)} / ${sizeLabel(total)}` : `正在下载：${sizeLabel(downloaded)}`;
  } else if (event.stage === 'extract') {
    progress.percent = 99;
    progress.stage = '正在解压并校验交付包';
  }
}

async function prepare() {
  if (controlsDisabled.value || !desktopAvailable || !candidate.value) return;
  const selected = candidate.value;
  operation.value = 'prepare';
  errorMessage.value = '';
  preparedBundle.value = null;
  emit('selected', null);
  Object.assign(progress, { percent: 0, total: 0, stage: `正在${preparationLabel.value}` });
  try {
    let bundle;
    if (selected.kind === 'release') {
      bundle = await downloadArtifact(selected.sourceId, selected.release, updateProgress);
    } else if (selected.kind === 'downloaded-version') {
      bundle = await openArtifactBundleVersion(selected.sourceId, selected.version);
    } else if (selected.kind === 'downloaded') {
      bundle = await openArtifactBundle(selected.sourceId);
    } else {
      bundle = await prepareLocalArtifactBundle(selected.path);
    }
    if (!bundle?.dir) throw new Error('交付包目录不存在');
    // Every source goes through the same installation checks before it becomes usable.
    if (selected.kind === 'release' || selected.kind === 'downloaded' || selected.kind === 'downloaded-version') {
      const validated = await prepareLocalArtifactBundle(bundle.dir);
      bundle = { ...bundle, ...validated };
    }
    preparedBundle.value = bundle;
    await applyProjectBundle(bundle.dir, bundle.version);
    Object.assign(progress, { percent: 100, stage: '校验完成' });
    emit('selected', bundle);
    if (selected.kind === 'release') {
      // Refresh local version metadata without turning a successful preparation
      // into a failure if refreshing the source list is unavailable.
      try { await readSources(); } catch { /* The validated bundle is usable. */ }
    }
  } catch (error) {
    progress.stage = '';
    errorMessage.value = artifactFailureText(error, preparationLabel.value);
  } finally {
    operation.value = '';
  }
}

defineExpose({ clearSelection, refreshSources });
onMounted(async () => {
  await refreshSources();
  await nextTick();
  applyPreset(initialPreset);
});
</script>

<style scoped>
.wizard-artifact-picker {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.selection-summary {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 16px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
}

.selection-summary > .el-icon {
  flex: none;
  color: var(--primary-color);
  font-size: 20px;
}

.selection-summary strong {
  font-weight: 600;
}

.selection-summary .field-hint {
  margin-top: 6px;
}

.field-hint,
.path-text {
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}

.path-text {
  overflow-wrap: anywhere;
}
</style>
