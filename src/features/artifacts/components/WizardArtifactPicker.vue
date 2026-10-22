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
      <section class="artifact-section">
        <div class="section-heading">
          <div>
            <h4>从制品源选择</h4>
            <p>使用已下载的交付包，或检查最新可用版本。</p>
          </div>
          <el-button link type="primary" :disabled="controlsDisabled" @click="emit('go-artifacts')">
            管理制品源
          </el-button>
        </div>

        <div class="source-controls">
          <el-select
            v-model="sourceId"
            placeholder="请选择制品源"
            :disabled="controlsDisabled || !desktopAvailable"
            aria-label="制品源"
          >
            <el-option v-for="source in sources" :key="source.id" :label="source.name" :value="source.id" />
          </el-select>
          <el-button
            :loading="operation === 'sources'"
            :disabled="controlsDisabled || !desktopAvailable"
            @click="refreshSources"
          >
            <el-icon><Refresh /></el-icon>
            <span>刷新</span>
          </el-button>
        </div>

        <p v-if="desktopAvailable && !sources.length && !busy" class="field-hint">
          尚未配置制品源，请先在设置的“制品设置”中添加，或选择本机制品。
        </p>
        <div v-if="selectedSource" class="source-actions">
          <el-button
            :disabled="controlsDisabled || !selectedSource.lastBundleDir"
            @click="chooseDownloaded"
          >
            使用已下载包{{ selectedSource.lastVersion && selectedSource.lastVersion !== 'manual' ? `（${selectedSource.lastVersion}）` : '' }}
          </el-button>
          <el-button
            :loading="operation === 'release'"
            :disabled="controlsDisabled"
            @click="checkLatest"
          >
            检查最新版本
          </el-button>
          <el-button
            :loading="operation === 'versions'"
            :disabled="controlsDisabled || !desktopAvailable"
            @click="toggleVersions"
          >
            {{ versionsVisible ? '收起版本列表' : '选择版本' }}
          </el-button>
          <span v-if="!selectedSource.lastBundleDir" class="field-hint">该制品源尚无已下载的交付包</span>
        </div>

        <div v-if="versionsVisible" class="version-list">
          <p v-if="!versions.length && operation !== 'versions'" class="field-hint">
            该制品源暂无可用版本。
          </p>
          <div v-for="item in versions" :key="releaseKey(item)" class="version-row">
            <div class="version-info">
              <div class="summary-line">
                <strong>{{ displayVersion(item.version) }}</strong>
                <el-tag v-if="item.commitShort" size="small" type="info" effect="plain">{{ item.commitShort }}</el-tag>
                <el-tag v-if="item.localDir" size="small" type="success" effect="plain">已下载</el-tag>
              </div>
              <p class="field-hint">
                构建时间：{{ formatTime(item.builtAt) }}
                <template v-if="item.sizeBytes">，大小：{{ formatSize(item.sizeBytes) }}</template>
              </p>
            </div>
            <el-button
              size="small"
              :type="isChosen(item) ? 'primary' : ''"
              :disabled="controlsDisabled || isChosen(item)"
              @click="chooseVersion(item)"
            >
              {{ isChosen(item) ? '已选择' : item.localDir ? '使用此版本' : '下载此版本' }}
            </el-button>
          </div>
          <el-button
            v-if="versionsHasMore"
            size="small"
            :loading="operation === 'versions'"
            :disabled="controlsDisabled"
            @click="loadVersions(false)"
          >
            加载更多版本
          </el-button>
        </div>

        <div v-if="release" class="release-summary">
          <div class="summary-line">
            <span class="summary-label">最新版本</span>
            <strong>{{ displayVersion(release.version) }}</strong>
            <el-tag v-if="release.commitShort" size="small" type="info" effect="plain">{{ release.commitShort }}</el-tag>
          </div>
          <p v-if="release.sizeBytes" class="field-hint">交付包大小：{{ formatSize(release.sizeBytes) }}</p>
          <el-button
            type="primary"
            :disabled="controlsDisabled || isChosen(release)"
            @click="chooseRelease"
          >
            {{ isChosen(release) ? '已选择' : '选择此版本' }}
          </el-button>
        </div>
      </section>

      <section class="artifact-section">
        <div class="section-heading">
          <div>
            <h4>选择本机制品</h4>
            <p>选择 .tgz、.tar.gz 交付包，或已解压的交付包目录。</p>
          </div>
        </div>
        <div class="local-actions">
          <el-button :disabled="controlsDisabled || !desktopAvailable" @click="pickLocal(false)">
            <el-icon><FolderOpened /></el-icon>
            <span>选择压缩包</span>
          </el-button>
          <el-button :disabled="controlsDisabled || !desktopAvailable" @click="pickLocal(true)">
            选择已解压目录
          </el-button>
          <el-button
            v-if="connection.bundleDir"
            :disabled="controlsDisabled || !desktopAvailable"
            @click="chooseCurrentDirectory"
          >
            使用当前交付包目录
          </el-button>
        </div>
      </section>

      <div v-if="candidate" class="selection-summary">
        <el-icon><DocumentChecked /></el-icon>
        <div>
          <strong>已选择：{{ candidate.label }}</strong>
          <p v-if="candidate.version" class="field-hint">版本：{{ displayVersion(candidate.version) }}</p>
          <p v-if="candidate.path" class="path-text">{{ candidate.path }}</p>
          <p class="field-hint">下一步将{{ preparationLabel }}，通过后读取部署清单。</p>
        </div>
      </div>
    </template>

    <template v-else>
      <div v-if="candidate" class="prepare-panel">
        <div class="selected-package">
          <span class="package-icon"><el-icon><Box /></el-icon></span>
          <div>
            <h4>{{ candidate.label }}</h4>
            <p v-if="candidate.version" class="field-hint">版本：{{ displayVersion(candidate.version) }}</p>
            <p v-if="candidate.path" class="path-text">{{ candidate.path }}</p>
          </div>
        </div>
        <p class="prepare-hint">{{ preparationHint }}</p>
        <el-button
          type="primary"
          :loading="preparing"
          :disabled="controlsDisabled || !desktopAvailable || Boolean(preparedBundle)"
          @click="prepare"
        >
          {{ preparedBundle ? '已校验完成' : preparationLabel }}
        </el-button>
        <div v-if="preparing || progress.stage" class="prepare-progress" role="status" aria-live="polite">
          <el-progress
            v-if="progress.total > 0 || preparedBundle"
            :percentage="progress.percent"
            :status="preparedBundle ? 'success' : undefined"
          />
          <p class="field-hint">{{ progress.stage }}</p>
        </div>
        <el-alert
          v-if="preparedBundle"
          title="交付包已准备就绪"
          :description="`已完成交付包校验，共 ${preparedBundle.files} 个文件，可继续读取部署清单。`"
          type="success"
          :closable="false"
          show-icon
        />
        <p v-if="preparedBundle" class="prepared-directory">交付包目录：{{ preparedBundle.dir }}</p>
      </div>
      <el-empty v-else description="请先返回上一步选择交付包" :image-size="72" />
    </template>

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
import { selectLocalPath as open } from '../../../api/tauri.js';
import { Box, DocumentChecked, FolderOpened, Refresh } from '@element-plus/icons-vue';
import { downloadArtifact, fetchArtifactRelease, isDesktopApp, listArtifactReleases, listArtifactSources, openArtifactBundle, openArtifactBundleVersion, prepareLocalArtifactBundle } from '../../../api/tauri.js';
import { applyProjectBundle } from '../../connections/state.js';

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
const preparationLabel = computed(() => {
  if (candidate.value?.kind === 'release') return '下载、解压并校验';
  if (candidate.value?.kind === 'archive') return '解压并校验';
  return '校验交付包';
});
const preparationHint = computed(() => {
  if (candidate.value?.kind === 'release') return '将下载所选版本，完成完整性校验后解压到本机。';
  if (candidate.value?.kind === 'archive') return '将交付包解压到本机，并检查内容是否完整。';
  return '检查交付包是否完整，以及部署清单是否可用。';
});

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
    label: preset.label || displayVersion(preset.version),
    version: preset.version || '',
    path
  });
}

function formatSize(bytes) {
  let size = Number(bytes || 0);
  if (!Number.isFinite(size) || size <= 0) return '未知大小';
  const units = ['B', 'KB', 'MB', 'GB'];
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function displayVersion(value) {
  return value && value !== 'manual' ? value : '未标记版本';
}

function failureText(error, action) {
  // The backend may include URLs or host paths in diagnostic text. Keep the
  // default message actionable without exposing source credentials or URLs.
  const reason = String(error || '');
  if (/令牌|鉴权|权限|401|403|token|unauthorized/i.test(reason)) {
    if (/目录|交付包/.test(action) && !/下载/.test(action)) {
      return '无法读取所选交付包，请确认文件访问权限后重试。';
    }
    return '无法访问制品源，请在设置的“制品设置”中检查访问凭证和权限后重试。';
  }
  if (/非法路径|符号链接|不安全|路径.*越界/.test(reason)) {
    return '交付包包含不安全的路径或链接，已停止解压，请重新获取可信的交付包。';
  }
  if (/缺少|必要文件|内容不完整|bundle\.json|格式无效|不是.*压缩包/.test(reason)) {
    return '交付包格式无效或内容不完整，请选择包含应用文件和部署清单的完整交付包。';
  }
  if (/校验|digest|checksum|md5|sha256/i.test(reason)) {
    return '交付包完整性校验未通过，请重新获取交付包后重试。';
  }
  if (/不存在|not found|no such file/i.test(reason)) {
    return '所选交付包已不存在，请重新选择或下载交付包。';
  }
  if (/超过.*限制|超过.*大小|体积|too large/i.test(reason)) {
    return '交付包超过允许大小，请选择符合大小要求的交付包。';
  }
  if (/网络|连接|超时|network|timeout|connect/i.test(reason)) {
    return '连接制品源失败，请检查网络与下载地址后重试。';
  }
  return `${action}失败，请确认所选制品可用后重试。`;
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
    errorMessage.value = failureText(error, '读取制品源');
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
    errorMessage.value = failureText(error, '检查最新版本');
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
    label: `${source.name}（${displayVersion(release.value.version)}）`,
    version: release.value.version,
    release: release.value
  });
}

function isChosen(item) {
  if (!item || !candidate.value) return false;
  if (candidate.value.sourceId !== sourceId.value) return false;
  const version = String(item.version || '').trim();
  return Boolean(version) && candidate.value.version === version;
}

function releaseKey(item) {
  return `${item.runId || 0}-${item.artifactName || ''}-${item.version || ''}`;
}

function formatTime(ms) {
  const time = Number(ms || 0);
  return time > 0 ? new Date(time).toLocaleString() : '未知时间';
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
    errorMessage.value = failureText(error, '读取版本列表');
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
      label: `${source.name}（${displayVersion(version)}）`,
      version,
      path: item.localDir
    });
    return;
  }
  choose({
    kind: 'release',
    sourceId: source.id,
    label: `${source.name}（${displayVersion(version)}）`,
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
    errorMessage.value = failureText(error, directory ? '选择交付包目录' : '选择交付包');
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
    progress.stage = total > 0 ? `正在下载：${formatSize(downloaded)} / ${formatSize(total)}` : `正在下载：${formatSize(downloaded)}`;
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
    errorMessage.value = failureText(error, preparationLabel.value);
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
.wizard-artifact-picker,
.artifact-section,
.prepare-panel {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.artifact-section {
  padding: 20px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--panel-bg);
}

.section-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.section-heading h4,
.selected-package h4 {
  font-size: 14px;
  line-height: 1.5;
}

.section-heading p,
.prepare-hint {
  margin-top: 4px;
  color: var(--text-muted);
  font-size: 13px;
  line-height: 1.6;
}

.source-controls,
.source-actions,
.local-actions,
.summary-line {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.source-controls .el-select {
  flex: 1;
  min-width: 180px;
}

.source-actions :deep(.el-button + .el-button),
.local-actions :deep(.el-button + .el-button) {
  margin-left: 0;
}

.release-summary,
.selection-summary {
  padding: 16px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
}

.version-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 16px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
}

.version-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 0;
  border-bottom: 1px solid var(--border-color);
}

.version-row:last-of-type {
  border-bottom: none;
}

.version-info {
  min-width: 0;
}

.version-list > .el-button {
  align-self: flex-start;
}

.release-summary {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 12px;
}

.summary-label {
  color: var(--text-muted);
}

.selection-summary,
.selected-package {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.selection-summary > .el-icon {
  flex: none;
  color: var(--primary-color);
  font-size: 20px;
}

.selection-summary strong {
  font-weight: 600;
}

.selection-summary .field-hint,
.selected-package .field-hint,
.selected-package .path-text {
  margin-top: 6px;
}

.package-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 42px;
  height: 42px;
  flex: none;
  border-radius: var(--radius-sm);
  background: var(--primary-soft);
  color: var(--primary-color);
  font-size: 22px;
}

.prepare-panel > .el-button {
  align-self: flex-start;
}

.field-hint,
.path-text,
.prepared-directory {
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}

.path-text,
.prepared-directory {
  overflow-wrap: anywhere;
}

.prepare-progress .field-hint {
  margin-top: 8px;
}

@media (max-width: 760px) {
  .artifact-section {
    padding: 16px;
  }

  .section-heading {
    flex-wrap: wrap;
  }
}
</style>
