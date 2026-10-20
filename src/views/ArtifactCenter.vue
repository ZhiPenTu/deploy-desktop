<template>
  <div class="page-view artifact-view">
    <div class="page-header">
      <div>
        <el-button v-if="activeSource" link class="back-button" @click="backToSources">
          <el-icon><ArrowLeft /></el-icon>
          <span>返回制品源管理</span>
        </el-button>
        <h2 class="page-title">{{ activeSource ? '制品列表' : '制品源管理' }}</h2>
        <p class="page-subtitle">
          {{ activeSource
            ? `查看 ${activeSource.name} 的制品、版本号和下载状态`
            : '选择制品源，查看可下载的制品及对应版本' }}
        </p>
      </div>
      <div class="header-actions">
        <el-tag type="info" effect="plain">当前部署包：{{ currentBundleText }}</el-tag>
        <el-button v-if="activeSource" :loading="refreshingActive" @click="refreshActive">
          <el-icon><Refresh /></el-icon>
          <span>刷新版本</span>
        </el-button>
        <template v-else>
          <el-button :loading="loading" @click="loadSources">
            <el-icon><Refresh /></el-icon>
            <span>刷新</span>
          </el-button>
          <el-button type="primary" @click="openCreate">
            <el-icon><Plus /></el-icon>
            <span>新建制品源</span>
          </el-button>
        </template>
      </div>
    </div>

    <template v-if="!activeSource">
      <div v-if="!loading && !sources.length" class="empty-panel card-surface">
        <span class="empty-icon"><el-icon><Box /></el-icon></span>
        <h3>尚未配置制品源</h3>
        <p>新建制品源后，可在该来源下查看最新版本与历史版本，下载并校验交付包。</p>
        <div class="empty-actions">
          <el-button type="primary" @click="openCreate">
            <el-icon><Plus /></el-icon>
            <span>新建制品源</span>
          </el-button>
          <el-button :loading="loading" @click="loadSources">
            <el-icon><Refresh /></el-icon>
            <span>刷新</span>
          </el-button>
        </div>
        <span class="empty-hint">令牌仅保存在本机，供当前用户读取</span>
      </div>

      <div v-else class="card-surface source-card">
        <div class="card-title-bar">
          <div>
            <h3>制品源</h3>
            <p class="card-subtitle">共 {{ sources.length }} 个来源，点击名称或“查看制品”进入版本列表</p>
          </div>
          <span class="hint">令牌仅保存在本机，供当前用户读取</span>
        </div>
        <el-table :data="sources" size="small" class="source-table">
          <el-table-column label="名称" min-width="220">
            <template #default="{ row }">
              <div class="name-cell">
                <el-button link class="source-name" @click="openSource(row)">
                  {{ row.name }}
                </el-button>
                <el-tag size="small" effect="plain">
                  {{ row.type === 'url' ? '直接下载' : '云效流水线' }}
                </el-tag>
                <el-tag v-if="isInUse(row)" size="small" type="success" effect="plain">
                  使用中
                </el-tag>
              </div>
            </template>
          </el-table-column>
          <el-table-column label="连接" min-width="190">
            <template #default="{ row }">
              <div v-if="row.type !== 'url'" class="stacked-cell">
                <span>流水线 #{{ row.pipelineId }}</span>
                <span class="subtle">组织 {{ row.orgId }}</span>
              </div>
              <span v-else class="subtle">直接下载地址</span>
            </template>
          </el-table-column>
          <el-table-column label="制品名" min-width="140">
            <template #default="{ row }">
              <code>{{ row.artifactName }}</code>
            </template>
          </el-table-column>
          <el-table-column label="本地版本" min-width="150">
            <template #default="{ row }">
              <div class="stacked-cell">
                <span :class="{ muted: !row.lastVersion }">{{ row.lastVersion || '尚未下载' }}</span>
                <span v-if="row.lastBundleDir" class="subtle">
                  {{ isInUse(row) ? '当前部署使用' : '已下载到本机' }}
                </span>
              </div>
            </template>
          </el-table-column>
          <el-table-column label="操作" width="210">
            <template #default="{ row }">
              <el-button link type="primary" @click="openSource(row)">查看制品</el-button>
              <el-button link @click="openEdit(row)">编辑</el-button>
              <el-button link type="danger" @click="remove(row)">删除</el-button>
            </template>
          </el-table-column>
        </el-table>
      </div>

      <div v-if="bundleDirText && (planLoading || plan)" class="plan-section">
        <div v-if="planLoading" v-loading="true" class="plan-loading card-surface">
          <span>正在读取交付包内容</span>
        </div>

        <BundlePlanPanel
          v-else-if="plan"
          :plan="plan"
          :target="remoteTarget"
          :image-dir="imageDir"
        >
          <template #actions>
            <el-button size="small" :loading="planLoading" @click="loadPlan">
              <el-icon><Refresh /></el-icon>
              <span>重新读取</span>
            </el-button>
            <el-button size="small" type="primary" @click="$emit('go-wizard')">
              进入部署向导
            </el-button>
            <el-button size="small" @click="$emit('go-update')">
              进入服务更新
            </el-button>
          </template>
        </BundlePlanPanel>
      </div>
    </template>

    <template v-else>
      <div class="source-overview card-surface">
        <div class="overview-main">
          <div class="overview-title-line">
            <h3>{{ activeSource.name }}</h3>
            <el-tag size="small" effect="plain">
              {{ activeSource.type === 'url' ? '直接下载' : '云效流水线' }}
            </el-tag>
          </div>
          <div class="overview-meta">
            <span v-if="activeSource.type === 'url'">来源：直接下载地址</span>
            <span v-else>来源：云效流水线 #{{ activeSource.pipelineId }}</span>
            <span>制品名：<code>{{ activeSource.artifactName }}</code></span>
            <span v-if="activeSource.lastVersion">本地版本：{{ activeSource.lastVersion }}</span>
          </div>
        </div>
        <div class="overview-actions">
          <el-select
            :model-value="activeSourceId"
            aria-label="选择制品源"
            class="source-switcher"
            @change="switchSource"
          >
            <el-option v-for="source in sources" :key="source.id" :label="source.name" :value="source.id" />
          </el-select>
          <el-button @click="openEdit(activeSource)">编辑制品源</el-button>
          <el-button
            v-if="activeSource.lastBundleDir"
            type="warning"
            plain
            :loading="removingId === activeSource.id"
            @click="removeLocal(activeSource)"
          >
            清理本地缓存
          </el-button>
        </div>
        <div class="overview-stats">
          <div class="overview-stat">
            <span class="stat-label">最新版本</span>
            <strong>{{ latestVersionText }}</strong>
          </div>
          <div class="overview-stat">
            <span class="stat-label">最近构建</span>
            <strong>{{ latestBuiltText }}</strong>
          </div>
          <div class="overview-stat">
            <span class="stat-label">已加载制品</span>
            <strong>{{ loadedVersionText }}</strong>
          </div>
        </div>
      </div>

      <div class="card-surface versions-card">
        <div class="card-title-bar">
          <div>
            <h3>制品列表</h3>
            <p class="card-subtitle">按构建时间倒序排列，最新版本置顶</p>
          </div>
          <el-button v-if="releases.length" link :loading="releasesLoading" @click="loadReleases({ reset: true })">
            <el-icon><Refresh /></el-icon>
            <span>刷新列表</span>
          </el-button>
        </div>

        <el-alert
          v-if="releasesError"
          type="warning"
          :closable="false"
          show-icon
          class="versions-alert"
        >
          <template #title>{{ releasesError }}</template>
          <div class="alert-actions">
            <span>请检查制品源配置后重试。</span>
            <el-button size="small" @click="loadReleases({ reset: true })">重试</el-button>
            <el-button size="small" @click="openEdit(activeSource)">编辑制品源</el-button>
          </div>
        </el-alert>

        <el-table
          v-if="releases.length"
          v-loading="releasesLoading"
          :data="releases"
          :row-key="releaseKey"
          size="small"
          class="versions-table"
        >
          <el-table-column label="制品名" min-width="160">
            <template #default="{ row }">
              <code>{{ row.artifactName || activeSource.artifactName }}</code>
            </template>
          </el-table-column>
          <el-table-column label="版本" min-width="175">
            <template #default="{ row }">
              <div class="version-cell">
                <div class="version-title">
                  <span class="version-number">{{ versionText(row) }}</span>
                  <el-tag v-if="isLatest(row)" size="small" type="primary" effect="plain">最新</el-tag>
                  <el-tag v-if="isCurrentBundle(row)" size="small" type="success" effect="plain">当前部署</el-tag>
                </div>
              </div>
            </template>
          </el-table-column>
          <el-table-column label="构建信息" min-width="260">
            <template #default="{ row }">
              <div v-if="row.runId" class="stacked-cell">
                <div class="commit-line">
                  <code>{{ row.commitShort || '-' }}</code>
                  <span class="commit-title">{{ row.commitTitle || '无提交说明' }}</span>
                </div>
                <span class="subtle">
                  运行 #{{ row.runId }}<template v-if="row.branch"> · 分支 {{ row.branch }}</template>
                </span>
              </div>
              <span v-else class="subtle">直接下载源，无构建信息</span>
            </template>
          </el-table-column>
          <el-table-column label="构建时间" width="180">
            <template #default="{ row }">{{ formatTime(row.builtAt) }}</template>
          </el-table-column>
          <el-table-column label="大小" width="100">
            <template #default="{ row }">{{ formatSize(row.sizeBytes) }}</template>
          </el-table-column>
          <el-table-column label="本地状态" width="120">
            <template #default="{ row }">
              <el-tag :type="localStatusType(row)" size="small" effect="plain">
                {{ localStatusText(row) }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column label="操作" min-width="200">
            <template #default="{ row }">
              <el-button
                v-if="row.localDir"
                link
                type="primary"
                :loading="loadingVersionKey === releaseKey(row)"
                :disabled="isCurrentBundle(row)"
                @click="useExisting(row)"
              >
                {{ isCurrentBundle(row) ? '使用中' : '使用' }}
              </el-button>
              <el-button v-else link type="primary" @click="openRelease(row)">下载</el-button>
              <el-button v-if="row.localDir" link @click="openRelease(row)">重新下载</el-button>
              <el-button v-if="row.runUrl" link @click="openRunUrl(row)">运行详情</el-button>
            </template>
          </el-table-column>
        </el-table>

        <div v-else-if="releasesLoading" class="versions-empty">
          <span class="empty-icon compact"><el-icon><Box /></el-icon></span>
          <h4>正在读取版本</h4>
          <p>正在获取该制品源的最新构建与历史记录。</p>
        </div>

        <div v-else-if="!releasesError" class="versions-empty">
          <span class="empty-icon compact"><el-icon><Box /></el-icon></span>
          <h4>当前未找到可用制品</h4>
          <p>{{ hasMore ? '可以继续加载更早的构建记录。' : '请确认流水线已成功构建，并上传了对应制品。' }}</p>
        </div>

        <div v-if="(releases.length || hasMore) && !releasesError" class="list-footer">
          <el-button v-if="hasMore" :loading="loadingMore" @click="loadMore">加载更早版本</el-button>
          <span v-else class="subtle">已显示全部可加载版本</span>
        </div>
      </div>
    </template>

    <ArtifactSourceEditor
      v-model="editorVisible"
      :source="editing"
      @saved="handleSourceSaved"
    />

    <el-dialog
      v-model="releaseVisible"
      :title="releaseDialogTitle"
      width="640px"
      :close-on-click-modal="!downloading"
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
          <span class="value">{{ formatTime(release.builtAt) }}</span>
          <span class="label">文件大小</span>
          <span class="value">{{ formatSize(release.sizeBytes) }}</span>
          <span class="label">MD5</span>
          <span class="value"><code>{{ release.md5 || '-' }}</code></span>
        </div>

        <el-progress
          v-if="downloading || progress.percent > 0"
          :percentage="progress.percent"
          :status="progress.percent >= 100 ? 'success' : undefined"
        />
        <p v-if="progress.stage" class="field-hint">{{ progress.stage }}</p>

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
        <el-button v-if="release && release.runUrl" link type="primary" @click="openRunUrl(release)">
          打开运行详情
        </el-button>
        <el-button @click="releaseVisible = false">关闭</el-button>
        <el-button v-if="bundle" type="primary" @click="applyBundle">设为当前部署包</el-button>
        <el-button v-else type="primary" :loading="downloading" @click="download">
          {{ release && release.localDir ? '重新下载并解包' : '下载并解包' }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref, watch } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { ArrowLeft, Box, Plus, Refresh } from '@element-plus/icons-vue';
import BundlePlanPanel from '../components/BundlePlanPanel.vue';
import ArtifactSourceEditor from '../components/ArtifactSourceEditor.vue';
import {
  deleteArtifactSource,
  deleteLocalBundle,
  downloadArtifact,
  listArtifactReleases,
  listArtifactSources,
  openArtifactBundleVersion
} from '../api/artifact.js';
import { inspectBundlePlan, openArtifactRunUrl } from '../api/tauri.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

defineEmits(['go-wizard', 'go-update']);

const sources = ref([]);
const loading = ref(false);
const refreshingActive = ref(false);
const removingId = ref('');
const editorVisible = ref(false);
const editing = ref(null);
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
const bundle = ref(null);
const progress = reactive({ percent: 0, stage: '' });
const plan = ref(null);
const planLoading = ref(false);
const imageDir = ref(localStorage.getItem('image-local-dir') || '');
let releaseRequestId = 0;

const activeSource = computed(
  () => sources.value.find((item) => item.id === activeSourceId.value) || null
);
const bundleDirText = computed(() => String(props.connection.bundleDir || '').trim());
const latestRelease = computed(() => releases.value[0] || null);
const latestVersionText = computed(() => {
  if (releasesLoading.value && !latestRelease.value) return '读取中';
  return latestRelease.value ? versionText(latestRelease.value) : '-';
});
const latestBuiltText = computed(() => {
  if (releasesLoading.value && !latestRelease.value) return '读取中';
  return latestRelease.value ? formatTime(latestRelease.value.builtAt) : '-';
});
const loadedVersionText = computed(() => {
  if (releasesLoading.value && !releases.value.length) return '读取中';
  return `${releases.value.length}${hasMore.value ? '+' : ''}`;
});
const releaseDialogTitle = computed(() =>
  release.value ? `下载版本 ${versionText(release.value)}` : '下载制品版本'
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

const currentBundleText = computed(() => {
  const dir = String(props.connection.bundleDir || '').trim();
  if (!dir) return '未设置';
  const parts = dir.split('/').filter(Boolean);
  return parts.slice(-2).join('/') || dir;
});

function versionText(item) {
  const version = String(item?.version || '').trim();
  if (!version || version === 'manual') return '未标注版本';
  return version;
}

function releaseKey(item) {
  return `${item.runId || 0}-${item.artifactName || ''}-${item.version || ''}`;
}

function isLatest(item) {
  return Boolean(item) && latestRelease.value === item;
}

function isInUse(row) {
  const current = String(props.connection.bundleDir || '').trim();
  const dir = String(row.lastBundleDir || '').trim();
  if (!current || !dir) return false;
  return current === dir || current.startsWith(`${dir}/`);
}

function isCurrentBundle(row) {
  const current = String(props.connection.bundleDir || '').trim();
  const dir = String(row.localDir || '').trim();
  if (!current || !dir) return false;
  return current === dir || current.startsWith(`${dir}/`);
}

function localStatusText(row) {
  if (isCurrentBundle(row)) return '当前使用';
  return row.localDir ? '已下载' : '未下载';
}

function localStatusType(row) {
  if (isCurrentBundle(row)) return 'success';
  return row.localDir ? 'primary' : 'info';
}

function shortPath(dir) {
  const parts = String(dir || '').split('/').filter(Boolean);
  if (parts.length <= 2) return dir;
  return `.../${parts.slice(-2).join('/')}`;
}

function formatSize(bytes) {
  const value = Number(bytes || 0);
  if (!value) return '-';
  const units = ['B', 'KB', 'MB', 'GB'];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function formatTime(ms) {
  if (!ms) return '-';
  return new Date(Number(ms)).toLocaleString();
}

async function loadPlan() {
  const dir = bundleDirText.value;
  if (!dir) {
    plan.value = null;
    return;
  }
  planLoading.value = true;
  try {
    plan.value = await inspectBundlePlan(dir, remoteTarget.value);
  } catch (error) {
    plan.value = null;
  } finally {
    planLoading.value = false;
  }
}

watch(bundleDirText, loadPlan, { immediate: true });

async function loadSources() {
  loading.value = true;
  try {
    sources.value = await listArtifactSources();
    if (activeSourceId.value && !activeSource.value) {
      activeSourceId.value = '';
    }
  } catch (error) {
    ElMessage.error(String(error));
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
    releases.value = [];
    releasePage.value = 0;
    hasMore.value = false;
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
      releasesError.value = String(error);
    } else {
      ElMessage.error(String(error));
    }
  } finally {
    if (requestId === releaseRequestId) {
      releasesLoading.value = false;
      loadingMore.value = false;
    }
  }
}

function openSource(row) {
  activeSourceId.value = row.id;
  releases.value = [];
  releasesError.value = '';
  releasePage.value = 0;
  hasMore.value = false;
  loadReleases({ reset: true });
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

function openCreate() {
  editing.value = null;
  editorVisible.value = true;
}

function openEdit(row) {
  editing.value = row;
  editorVisible.value = true;
}

async function handleSourceSaved() {
  await loadSources();
  if (activeSource.value) {
    await loadReleases({ reset: true });
  }
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
      props.connection.bundleDir = '';
    }
    ElMessage.success(`已清理本地交付包，释放 ${formatSize(result.freedBytes)}`);
    await loadSources();
    if (activeSource.value) {
      await loadReleases({ reset: true });
    }
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    removingId.value = '';
  }
}

async function remove(row) {
  try {
    await ElMessageBox.confirm(`确认删除制品源「${row.name}」？本地已下载的交付包不会被删除。`, '删除制品源', {
      type: 'warning'
    });
  } catch {
    return;
  }
  try {
    await deleteArtifactSource(row.id);
    ElMessage.success('已删除');
    await loadSources();
  } catch (error) {
    ElMessage.error(String(error));
  }
}

function openRelease(row) {
  releaseSource.value = activeSource.value;
  release.value = row;
  bundle.value = null;
  progress.percent = 0;
  progress.stage = '';
  releaseVisible.value = true;
}

async function download() {
  if (!release.value || !releaseSource.value) return;
  downloading.value = true;
  progress.percent = 0;
  progress.stage = '准备下载';
  bundle.value = null;
  try {
    const result = await downloadArtifact(releaseSource.value.id, release.value, (event) => {
      if (event.stage === 'download') {
        const total = Number(event.total || 0);
        const done = Number(event.downloaded || 0);
        progress.percent = total > 0 ? Math.min(99, Math.round((done / total) * 100)) : 0;
        progress.stage = `下载中 ${formatSize(done)} / ${formatSize(total)}`;
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
    ElMessage.error(String(error));
  } finally {
    downloading.value = false;
  }
}

async function useExisting(row) {
  const source = activeSource.value;
  if (!source) return;
  loadingVersionKey.value = releaseKey(row);
  try {
    const result = await openArtifactBundleVersion(source.id, row.version || 'manual');
    props.connection.bundleDir = result.dir;
    ElMessage.success(`已使用本地交付包 ${versionText(result)}`.trim());
    await loadSources();
    if (activeSource.value) {
      await loadReleases({ reset: true });
    }
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    loadingVersionKey.value = '';
  }
}

function applyBundle() {
  if (!bundle.value) return;
  props.connection.bundleDir = bundle.value.dir;
  ElMessage.success('已设为当前部署包');
  releaseVisible.value = false;
}

async function openRunUrl(item) {
  const url = item?.runUrl;
  if (!url) return;
  try {
    await openArtifactRunUrl(url);
  } catch (error) {
    ElMessage.error(String(error));
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

.source-card,
.versions-card {
  padding: 20px;
}

.source-table,
.versions-table {
  margin-top: 4px;
}

.source-table :deep(.el-button + .el-button),
.versions-table :deep(.el-button + .el-button) {
  margin-left: 8px;
}

.source-table :deep(.cell),
.versions-table :deep(.cell) {
  white-space: nowrap;
}

.name-cell,
.version-title,
.commit-line {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.source-name {
  min-width: 0;
  max-width: 230px;
  justify-content: flex-start;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}

.stacked-cell,
.version-cell {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.subtle {
  color: var(--text-muted);
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.commit-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
}

.version-number {
  color: var(--text-main);
  font-size: 14px;
  font-weight: 650;
}

.source-overview {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 18px 24px;
  padding: 20px;
}

.overview-title-line,
.overview-meta,
.overview-actions,
.overview-stats,
.alert-actions,
.list-footer {
  display: flex;
  align-items: center;
}

.overview-title-line {
  gap: 10px;
}

.overview-title-line h3 {
  font-size: 17px;
  font-weight: 650;
}

.overview-meta {
  flex-wrap: wrap;
  gap: 6px 18px;
  margin-top: 9px;
  color: var(--text-muted);
  font-size: 12px;
}

.overview-actions {
  gap: 8px;
  align-self: start;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.source-switcher {
  width: 200px;
}

.overview-stats {
  grid-column: 1 / -1;
  gap: 10px;
  padding-top: 16px;
  border-top: 1px solid var(--border-color);
}

.overview-stat {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}

.stat-label {
  color: var(--text-muted);
  font-size: 11px;
}

.overview-stat strong {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-main);
  font-size: 13px;
  font-weight: 600;
}

.card-title-bar {
  align-items: flex-start;
}

.card-subtitle {
  margin-top: 5px;
  color: var(--text-muted);
  font-size: 12px;
}

.versions-alert {
  margin-bottom: 14px;
}

.alert-actions {
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 6px;
}

.versions-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 36px 24px;
  text-align: center;
}

.versions-empty h4 {
  margin-top: 12px;
  color: var(--text-main);
  font-size: 14px;
  font-weight: 600;
}

.versions-empty p {
  max-width: 420px;
  margin: 7px 0 14px;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}

.list-footer {
  justify-content: center;
  min-height: 46px;
  margin-top: 10px;
  padding-top: 12px;
  border-top: 1px solid var(--border-color);
  color: var(--text-muted);
  font-size: 12px;
}

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

.muted {
  color: var(--text-muted);
  word-break: break-all;
}

.empty-panel {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  padding: 56px 32px 48px;
  background:
    radial-gradient(420px 180px at 50% 0%, rgba(51, 92, 255, 0.06), transparent 70%),
    var(--panel-bg);
}

.empty-icon {
  width: 52px;
  height: 52px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 14px;
  background: var(--primary-soft);
  color: var(--primary-color);
  font-size: 24px;
  margin-bottom: 18px;
}

.empty-icon.compact {
  width: 42px;
  height: 42px;
  margin-bottom: 0;
  border-radius: 12px;
  font-size: 20px;
}

.empty-panel h3 {
  font-size: 16px;
  font-weight: 600;
  color: var(--text-main);
}

.empty-panel p {
  max-width: 440px;
  margin-top: 8px;
  font-size: 12.5px;
  line-height: 1.65;
  color: var(--text-muted);
}

.empty-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 22px;
}

.empty-hint {
  margin-top: 16px;
  font-size: 11px;
  color: var(--text-faint);
}

.plan-section {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.plan-loading {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 140px;
  font-size: 13px;
  color: var(--text-muted);
}

@media (max-width: 980px) {
  .source-overview {
    grid-template-columns: 1fr;
  }

  .overview-actions {
    align-self: stretch;
  }

  .overview-stats {
    flex-wrap: wrap;
  }

  .overview-stat {
    flex-basis: 180px;
  }
}
</style>
