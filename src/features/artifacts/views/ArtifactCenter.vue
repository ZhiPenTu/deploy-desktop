<template>
  <div class="page-view artifact-view">
    <div class="page-header">
      <div>
        <el-button v-if="activeSource" link class="back-button" @click="backToSources">
          <el-icon><ArrowLeft /></el-icon>
          <span>返回制品源列表</span>
        </el-button>
        <h2 class="page-title">{{ activeSource ? '制品列表' : '选择制品源' }}</h2>
        <p class="page-subtitle">
          {{ activeSource
            ? `查看 ${activeSource.name} 的制品、版本号和下载状态`
            : '选择已配置的制品源，查看可下载的制品及对应版本' }}
        </p>
      </div>
      <div class="header-actions">
        <el-button v-if="activeSource" :loading="refreshingActive" @click="refreshActive">
          <el-icon><Refresh /></el-icon>
          <span>刷新版本</span>
        </el-button>
        <template v-else>
          <el-button :loading="loading" @click="loadSources">
            <el-icon><Refresh /></el-icon>
            <span>刷新</span>
          </el-button>
          <el-button @click="goSourceSettings">前往设置</el-button>
        </template>
      </div>
    </div>

    <ArtifactSourceList
      v-if="!activeSource"
      :sources="sources"
      :loading="loading"
      @open-source="openSource"
      @refresh="loadSources"
      @go-settings="goSourceSettings"
    />

    <template v-else>
      <ArtifactSourceOverview
        :source="activeSource"
        :sources="sources"
        :active-source-id="activeSourceId"
        :removing="removingId === activeSource.id"
        :releases="releases"
        :releases-loading="releasesLoading"
        :has-more="hasMore"
        @switch-source="switchSource"
        @remove-local="removeLocal"
      />

      <ReleaseListPanel
        :releases="releases"
        :loading="releasesLoading"
        :error="releasesError"
        :has-more="hasMore"
        :loading-more="loadingMore"
        :source-type="activeSource.type"
        @refresh="loadReleases({ reset: true })"
        @go-settings="goSourceSettings"
        @load-more="loadMore"
        @show-detail="showReleaseDetail"
        @download="openRelease"
        @open-run="openRunUrl"
      />
    </template>

    <ReleaseDownloadDialog
      v-model:visible="releaseVisible"
      :release="release"
      :downloading="downloading"
      :progress="progress"
      :download-error="downloadError"
      :bundle="bundle"
      @download="download"
      @open-run="openRunUrl"
      @open-logs="openRuntimeLogs"
      @deploy="deployBundle"
    />

    <ReleaseDetailDialog
      v-model:visible="detailVisible"
      :release="detailRelease"
      :plan="detailPlan"
      :loading="detailLoading"
      :error="detailError"
      :target="remoteTarget"
      :deploying="loadingVersionKey === releaseKey(detailRelease)"
      @closed="resetDetail"
      @open-run="openRunUrl"
      @deploy="deployLocalVersion"
      @download="downloadFromDetail"
    />
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { ArrowLeft, Refresh } from '@element-plus/icons-vue';
import ArtifactSourceList from '../components/ArtifactSourceList.vue';
import ArtifactSourceOverview from '../components/ArtifactSourceOverview.vue';
import ReleaseListPanel from '../components/ReleaseListPanel.vue';
import ReleaseDetailDialog from '../components/ReleaseDetailDialog.vue';
import ReleaseDownloadDialog from '../components/ReleaseDownloadDialog.vue';
import {
  deleteLocalBundle,
  downloadArtifact,
  listArtifactReleases,
  listCachedArtifactReleases,
  listArtifactSources,
  openArtifactBundleVersion,
  inspectBundlePlan,
  openArtifactRunUrl
} from '../../../api/tauri.js';
import { reloadProjectState } from '../../connections/state.js';
import { errorText } from '../../../shared/errors.js';
import {
  artifactSizeText,
  isBundleDirInUse,
  releaseKey,
  versionText
} from '../domain/releases.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const emit = defineEmits(['go-wizard', 'go-settings']);

const sources = ref([]);
const loading = ref(false);
const refreshingActive = ref(false);
const removingId = ref('');
const activeSourceId = ref('');
const releases = ref([]);
const releasesLoading = ref(false);
const releasesError = ref('');
const releasePage = ref(0);
const hasMore = ref(false);
const loadingMore = ref(false);
const loadingVersionKey = ref('');
const releaseVisible = ref(false);
const release = ref(null);
const releaseSource = ref(null);
const downloading = ref(false);
const downloadError = ref('');
const bundle = ref(null);
const progress = reactive({ percent: 0, stage: '', phase: '' });
const detailVisible = ref(false);
const detailRelease = ref(null);
const detailPlan = ref(null);
const detailLoading = ref(false);
const detailError = ref('');
let releaseRequestId = 0;
let detailRequestId = 0;

const activeSource = computed(
  () => sources.value.find((item) => item.id === activeSourceId.value) || null
);
const remoteTarget = computed(() => {
  const connection = props.connection || {};
  if (connection.mode !== 'remote' || !connection.remoteHost) return null;
  return {
    host: connection.remoteHost,
    port: connection.remotePort,
    user: connection.remoteUser,
    keyPath: connection.remoteKey || null
  };
});

function isInUse(row) {
  return isBundleDirInUse(props.connection.bundleDir, row.lastBundleDir);
}

async function showReleaseDetail(row) {
  detailRequestId += 1;
  detailRelease.value = row;
  detailVisible.value = true;
  await loadDetailPlan();
}

async function loadDetailPlan() {
  const row = detailRelease.value;
  const dir = String(row?.localDir || '').trim();
  const requestId = ++detailRequestId;
  detailPlan.value = null;
  detailError.value = '';
  if (!dir) {
    detailLoading.value = false;
    return;
  }
  detailLoading.value = true;
  try {
    const result = await inspectBundlePlan(dir, remoteTarget.value);
    if (requestId !== detailRequestId) return;
    detailPlan.value = result;
  } catch (error) {
    if (requestId !== detailRequestId) return;
    detailError.value = errorText(error);
  } finally {
    if (requestId === detailRequestId) {
      detailLoading.value = false;
    }
  }
}

function resetDetail() {
  detailRequestId += 1;
  detailRelease.value = null;
  detailPlan.value = null;
  detailError.value = '';
  detailLoading.value = false;
}

function downloadFromDetail() {
  const row = detailRelease.value;
  if (!row) return;
  detailVisible.value = false;
  openRelease(row);
}

async function loadSources() {
  loading.value = true;
  try {
    sources.value = await listArtifactSources();
    if (activeSourceId.value && !activeSource.value) {
      activeSourceId.value = '';
    }
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    loading.value = false;
  }
}

async function loadReleases({ reset = false } = {}) {
  const source = activeSource.value;
  if (!source) return;
  const requestId = ++releaseRequestId;
  const sourceId = source.id;

  if (reset) {
    releasesLoading.value = true;
    releasesError.value = '';
  } else {
    loadingMore.value = true;
  }

  try {
    const nextPage = reset ? 1 : releasePage.value + 1;
    const result = await listArtifactReleases(sourceId, nextPage);
    if (requestId !== releaseRequestId || activeSourceId.value !== sourceId) return;

    if (reset) {
      releases.value = result.items || [];
    } else {
      const known = new Set(releases.value.map(releaseKey));
      const appended = (result.items || []).filter((item) => !known.has(releaseKey(item)));
      releases.value = [...releases.value, ...appended];
    }
    releasePage.value = result.page || nextPage;
    hasMore.value = Boolean(result.hasMore);
  } catch (error) {
    if (requestId !== releaseRequestId || activeSourceId.value !== sourceId) return;
    if (reset) {
      releasesError.value = errorText(error);
    } else {
      ElMessage.error(errorText(error));
    }
  } finally {
    if (requestId === releaseRequestId) {
      releasesLoading.value = false;
      loadingMore.value = false;
    }
  }
}

async function loadCachedReleases() {
  const source = activeSource.value;
  if (!source) return;
  const requestId = ++releaseRequestId;
  const sourceId = source.id;
  releasesLoading.value = true;
  try {
    const result = await listCachedArtifactReleases(sourceId);
    if (requestId !== releaseRequestId || activeSourceId.value !== sourceId) return;
    releases.value = result.items || [];
    releasePage.value = result.page || 1;
    hasMore.value = Boolean(result.hasMore);
  } catch {
    // 本地缓存不可读时继续请求在线列表，不在这里中断页面。
  }
}

async function openSource(row) {
  activeSourceId.value = row.id;
  releases.value = [];
  releasesError.value = '';
  releasePage.value = 0;
  hasMore.value = false;
  await loadCachedReleases();
  if (activeSourceId.value !== row.id) return;
  await loadReleases({ reset: true });
}

function backToSources() {
  releaseRequestId += 1;
  activeSourceId.value = '';
  releases.value = [];
  releasesError.value = '';
  releasesLoading.value = false;
  loadingMore.value = false;
  releasePage.value = 0;
  hasMore.value = false;
}

function switchSource(id) {
  const source = sources.value.find((item) => item.id === id);
  if (source) openSource(source);
}

async function refreshActive() {
  refreshingActive.value = true;
  try {
    await loadSources();
    if (activeSource.value) {
      await loadReleases({ reset: true });
    }
  } finally {
    refreshingActive.value = false;
  }
}

function loadMore() {
  if (!hasMore.value || loadingMore.value) return;
  loadReleases({ reset: false });
}

async function removeLocal(row) {
  const inUse = isInUse(row);
  const message = inUse
    ? `确认删除该制品源的本地交付包？\n\n${row.lastBundleDir}\n\n该包当前正在使用，删除后需要重新下载并选择交付包。`
    : `确认删除该制品源的本地交付包？\n\n${row.lastBundleDir}\n\n本地缓存的全部版本都会被删除，云端制品不受影响。`;
  try {
    await ElMessageBox.confirm(message, '清理本地缓存', {
      type: 'warning',
      confirmButtonText: '删除',
      cancelButtonText: '取消'
    });
  } catch {
    return;
  }

  removingId.value = row.id;
  try {
    const result = await deleteLocalBundle(row.id);
    if (inUse) {
      await reloadProjectState();
    }
    ElMessage.success(`已清理本地交付包，释放 ${artifactSizeText(result.freedBytes)}`);
    await loadSources();
    if (activeSource.value) {
      await loadReleases({ reset: true });
    }
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    removingId.value = '';
  }
}

function openRelease(row) {
  releaseSource.value = activeSource.value;
  release.value = row;
  bundle.value = null;
  downloadError.value = '';
  progress.percent = 0;
  progress.stage = '';
  progress.phase = '';
  releaseVisible.value = true;
}

async function download() {
  if (!release.value || !releaseSource.value) return;
  downloading.value = true;
  progress.percent = 0;
  progress.stage = '准备下载';
  progress.phase = 'prepare';
  bundle.value = null;
  downloadError.value = '';
  try {
    const result = await downloadArtifact(releaseSource.value.id, release.value, (event) => {
      progress.phase = event.stage;
      if (event.stage === 'download') {
        const total = Number(event.total || 0);
        const done = Number(event.downloaded || 0);
        progress.percent = total > 0 ? Math.min(99, Math.round((done / total) * 100)) : 0;
        progress.stage = `下载中 ${artifactSizeText(done)} / ${artifactSizeText(total)}`;
      } else if (event.stage === 'extract') {
        progress.percent = 99;
        progress.stage = '校验并解包';
      } else if (event.stage === 'done') {
        progress.percent = 100;
        progress.stage = '完成';
      }
    });
    bundle.value = result;
    progress.percent = 100;
    progress.stage = '完成';
    ElMessage.success('交付包已下载并校验通过');
    await loadSources();
    if (activeSource.value) {
      await loadReleases({ reset: true });
    }
  } catch (error) {
    downloadError.value = errorText(error);
    ElMessage.error(errorText(error));
    progress.percent = 0;
    progress.stage = '';
    progress.phase = '';
  } finally {
    downloading.value = false;
  }
}

/** 把交付包带入部署向导，由向导完成环境检测、清单比对与部署确认。 */
function openDeployment(dir, version, sourceName) {
  const bundleDir = String(dir || '').trim();
  if (!bundleDir) return;
  const versionLabel = versionText({ version });
  emit('go-wizard', {
    dir: bundleDir,
    version: String(version || '').trim(),
    label: sourceName ? `${sourceName}（${versionLabel}）` : versionLabel
  });
  ElMessage.success('已选择该版本，请在部署向导中确认目标后执行部署');
}

async function deployLocalVersion(row) {
  const source = activeSource.value;
  if (!source || !row?.localDir) return;
  loadingVersionKey.value = releaseKey(row);
  try {
    const result = await openArtifactBundleVersion(source.id, row.version || 'manual');
    openDeployment(result.dir, result.version || row.version || '', source.name);
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    loadingVersionKey.value = '';
  }
}

function goSourceSettings() {
  emit('go-settings', 'artifacts');
}

function openRuntimeLogs() {
  emit('go-settings', 'logs');
}

function deployBundle() {
  if (!bundle.value) return;
  releaseVisible.value = false;
  openDeployment(bundle.value.dir, bundle.value.version, releaseSource.value?.name || activeSource.value?.name);
}

async function openRunUrl(item) {
  const url = item?.runUrl;
  if (!url) return;
  try {
    await openArtifactRunUrl(url);
  } catch (error) {
    ElMessage.error(errorText(error));
  }
}

onMounted(loadSources);
</script>

<style scoped>
.artifact-view {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.back-button {
  margin-bottom: 8px;
  padding: 0;
  color: var(--text-secondary);
}
</style>
