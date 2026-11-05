<template>
  <el-dialog
    :model-value="visible"
    title="下载 Docker 离线包"
    width="640px"
    append-to-body
    :close-on-click-modal="!downloading"
    :close-on-press-escape="!downloading"
    :show-close="!downloading"
    @update:model-value="onVisibleChange"
    @closed="reset"
  >
    <div v-loading="loading" class="download-body">
      <el-alert
        v-if="loadError"
        :title="loadError"
        type="error"
        :closable="false"
        show-icon
      >
        <el-button size="small" @click="loadPackages">重新获取</el-button>
      </el-alert>

      <template v-else>
        <el-alert
          v-if="warnings.length"
          :title="warnings.join('；')"
          type="warning"
          :closable="false"
          show-icon
        />

        <el-form label-width="88px" :disabled="downloading">
          <el-form-item label="适用平台">
            <el-select v-model="platformKey" style="width: 100%" @change="onPlatformChange">
              <el-option
                v-for="option in platformOptions"
                :key="option.value"
                :label="option.label"
                :value="option.value"
              />
            </el-select>
          </el-form-item>
          <el-form-item label="版本">
            <el-select
              v-model="packageKey"
              filterable
              style="width: 100%"
              placeholder="选择版本"
              no-data-text="暂无可下载版本"
              @change="onPackageChange"
            >
              <el-option
                v-for="item in platformPackages"
                :key="item.key"
                :label="versionLabel(item)"
                :value="item.key"
              />
            </el-select>
          </el-form-item>
          <el-form-item label="保存位置">
            <div class="save-row">
              <el-input
                :model-value="savePath"
                readonly
                placeholder="下载时选择保存位置"
              />
              <el-button :disabled="downloading" @click="pickSavePath">选择位置</el-button>
            </div>
          </el-form-item>
        </el-form>

        <div v-if="selectedPackage" class="package-summary">
          <div>
            <span>文件名</span>
            <strong>{{ selectedPackage.fileName }}</strong>
          </div>
          <div>
            <span>版本</span>
            <strong>{{ versionLabel(selectedPackage) }}</strong>
          </div>
          <div>
            <span>下载来源</span>
            <strong>Docker 官方</strong>
          </div>
        </div>

        <p v-if="selectedPackage" class="field-hint">{{ packageHint }}</p>

        <el-progress
          v-if="downloading || percent > 0"
          :percentage="percent"
          :status="result ? 'success' : undefined"
        />
        <p v-if="progressText" class="progress-text">{{ progressText }}</p>

        <el-alert
          v-if="downloadError"
          :title="downloadError"
          type="error"
          :closable="false"
          show-icon
        />
        <el-alert
          v-if="result"
          title="离线包已下载"
          :description="`已保存到 ${result.path}`"
          type="success"
          :closable="false"
          show-icon
        />
      </template>
    </div>

    <template #footer>
      <el-button :disabled="downloading" @click="close">关闭</el-button>
      <el-button
        type="primary"
        :loading="downloading"
        :disabled="!selectedPackage || loading || Boolean(loadError)"
        @click="download"
      >
        下载
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import {
  downloadDockerOfflinePackage,
  listDockerOfflinePackages,
  selectLocalSavePath
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';
import { formatSize } from '../../../shared/format.js';

const PLATFORM_LABELS = {
  'linux:x86_64': 'Linux（x86_64）',
  'linux:aarch64': 'Linux（ARM64）',
  'windows:amd64': 'Windows（x64）',
  'macos:arm64': 'macOS（Apple 芯片）',
  'macos:amd64': 'macOS（Intel 芯片）'
};
const PLATFORM_ORDER = [
  'linux:x86_64',
  'linux:aarch64',
  'windows:amd64',
  'macos:arm64',
  'macos:amd64'
];

const props = defineProps({
  visible: { type: Boolean, default: false },
  mediaDir: { type: String, default: '' },
  isRemote: { type: Boolean, default: false }
});

const emit = defineEmits(['update:visible', 'downloaded']);

const loading = ref(false);
const downloading = ref(false);
const loadError = ref('');
const downloadError = ref('');
const warnings = ref([]);
const packages = ref([]);
const platformKey = ref('');
const packageKey = ref('');
const savePath = ref('');
const downloadDir = ref('');
const downloadedBytes = ref(0);
const totalBytes = ref(0);
const progressText = ref('');
const result = ref(null);

const platformOptions = computed(() => {
  const present = new Set(packages.value.map((item) => item.platformKey));
  const ordered = PLATFORM_ORDER.filter((value) => present.has(value));
  for (const value of present) {
    if (!ordered.includes(value)) ordered.push(value);
  }
  return ordered.map((value) => ({
    value,
    label: PLATFORM_LABELS[value] || value
  }));
});

const platformPackages = computed(() =>
  packages.value.filter((item) => item.platformKey === platformKey.value)
);

const selectedPackage = computed(
  () => platformPackages.value.find((item) => item.key === packageKey.value) || null
);

const percent = computed(() => {
  if (result.value) return 100;
  if (!totalBytes.value) return 0;
  return Math.min(99, Math.round((downloadedBytes.value / totalBytes.value) * 100));
});

const packageHint = computed(() => {
  if (!selectedPackage.value) return '';
  if (selectedPackage.value.platform === 'linux') {
    return props.isRemote
      ? '下载到本机后，请把 Linux 包放入目标主机离线介质目录的 docker-engine 子目录，再执行离线安装。'
      : 'Linux 包放入离线介质目录的 docker-engine 子目录后，可在上方执行离线安装。';
  }
  return '下载后运行安装包完成安装；当前应用仅支持在 Linux 目标机执行离线安装。';
});

const defaultSavePath = computed(() => {
  const item = selectedPackage.value;
  if (!item) return '';
  const baseDir = (props.isRemote ? '' : props.mediaDir.trim()) || downloadDir.value;
  if (!baseDir) return item.fileName;
  if (item.platform === 'linux') {
    return `${baseDir}/docker-engine/${item.fileName}`;
  }
  return `${baseDir}/${item.fileName}`;
});

watch(
  () => props.visible,
  (visible) => {
    if (visible) loadPackages();
  }
);

function packageKeyOf(item) {
  return `${item.platform}:${item.arch}:${item.version}:${item.build}`;
}

function platformKeyOf(item) {
  return `${item.platform}:${item.arch}`;
}

function versionLabel(item) {
  return item.build ? `${item.version}（构建 ${item.build}）` : item.version;
}

async function loadPackages() {
  if (loading.value) return;
  loading.value = true;
  loadError.value = '';
  downloadError.value = '';
  warnings.value = [];
  packages.value = [];
  platformKey.value = '';
  packageKey.value = '';
  savePath.value = '';
  result.value = null;
  downloadedBytes.value = 0;
  totalBytes.value = 0;
  progressText.value = '';

  try {
    const response = await listDockerOfflinePackages();
    const items = (response?.packages || []).map((item) => ({
      ...item,
      platformKey: platformKeyOf(item),
      key: packageKeyOf(item)
    }));
    if (!items.length) throw new Error('暂时没有可下载的 Docker 离线包，请稍后重试');
    packages.value = items;
    warnings.value = response.warnings || [];
    downloadDir.value = response.downloadDir || '';
    const preferred = response.defaultPlatform;
    platformKey.value = platformOptions.value.some((option) => option.value === preferred)
      ? preferred
      : platformOptions.value[0]?.value || '';
    selectFirstVersion();
  } catch (error) {
    loadError.value = errorText(error);
  } finally {
    loading.value = false;
  }
}

function selectFirstVersion() {
  packageKey.value = platformPackages.value[0]?.key || '';
  savePath.value = '';
  result.value = null;
  downloadError.value = '';
  downloadedBytes.value = 0;
  totalBytes.value = 0;
  progressText.value = '';
}

function onPlatformChange() {
  selectFirstVersion();
}

function onPackageChange() {
  savePath.value = '';
  result.value = null;
  downloadError.value = '';
  downloadedBytes.value = 0;
  totalBytes.value = 0;
  progressText.value = '';
}

async function pickSavePath() {
  const item = selectedPackage.value;
  if (!item) return '';
  try {
    const selected = await selectLocalSavePath({
      title: '保存 Docker 离线包',
      defaultPath: defaultSavePath.value,
      filters: [
        {
          name: item.platform === 'linux' ? 'Docker 离线包' : 'Docker 安装包',
          extensions: [item.fileName.split('.').pop()]
        }
      ]
    });
    if (typeof selected === 'string' && selected) {
      savePath.value = selected;
    }
  } catch (error) {
    ElMessage.error(errorText(error));
  }
  return savePath.value;
}

async function download() {
  const item = selectedPackage.value;
  if (!item || downloading.value) return;
  if (!savePath.value) {
    const selected = await pickSavePath();
    if (!selected) return;
  }

  downloading.value = true;
  downloadError.value = '';
  result.value = null;
  downloadedBytes.value = 0;
  totalBytes.value = 0;
  progressText.value = '正在连接 Docker 官方下载站';

  try {
    const response = await downloadDockerOfflinePackage(item, savePath.value, (event) => {
      if (event.stage === 'done') {
        progressText.value = '下载完成，正在保存文件';
        return;
      }
      downloadedBytes.value = Number(event.downloaded) || 0;
      totalBytes.value = Number(event.total) || 0;
      progressText.value = totalBytes.value
        ? `正在下载 ${formatSize(downloadedBytes.value)} / ${formatSize(totalBytes.value)}`
        : `已下载 ${formatSize(downloadedBytes.value)}`;
    });
    result.value = response;
    savePath.value = response.path;
    progressText.value = `下载完成，共 ${formatSize(response.sizeBytes)}`;
    emit('downloaded', response);
    ElMessage.success('Docker 离线包下载完成');
  } catch (error) {
    downloadError.value = errorText(error);
    progressText.value = '';
  } finally {
    downloading.value = false;
  }
}

function onVisibleChange(value) {
  if (!value && downloading.value) return;
  emit('update:visible', value);
}

function close() {
  if (downloading.value) return;
  emit('update:visible', false);
}

function reset() {
  if (downloading.value) return;
  loading.value = false;
  loadError.value = '';
  downloadError.value = '';
  warnings.value = [];
  packages.value = [];
  platformKey.value = '';
  packageKey.value = '';
  savePath.value = '';
  downloadDir.value = '';
  downloadedBytes.value = 0;
  totalBytes.value = 0;
  progressText.value = '';
  result.value = null;
}
</script>

<style scoped>
.download-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-height: 180px;
}

.save-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
}

.save-row .el-input {
  flex: 1;
}

.package-summary {
  display: grid;
  gap: 8px;
  padding: 14px 16px;
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
}

.package-summary > div {
  display: grid;
  grid-template-columns: 72px 1fr;
  gap: 12px;
  align-items: start;
}

.package-summary span {
  color: var(--text-muted);
  font-size: 12px;
}

.package-summary strong {
  color: var(--text-main);
  font-size: 12.5px;
  font-weight: 600;
  overflow-wrap: anywhere;
}

.progress-text {
  margin: 0;
  color: var(--text-secondary);
  font-size: 12.5px;
  line-height: 1.6;
}
</style>
