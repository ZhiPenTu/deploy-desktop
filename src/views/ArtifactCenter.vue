<template>
  <div class="page-view artifact-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">制品中心</h2>
        <p class="page-subtitle">从云效流水线获取构建产物，校验后作为部署与更新使用的交付包，并展示该包从制品到容器的完整链路</p>
      </div>
      <div class="header-actions">
        <el-tag type="info" effect="plain">当前部署包：{{ currentBundleText }}</el-tag>
        <el-button :loading="loading" @click="loadSources">
          <el-icon><Refresh /></el-icon>
          <span>刷新</span>
        </el-button>
        <el-button type="primary" @click="openCreate">
          <el-icon><Plus /></el-icon>
          <span>新建制品源</span>
        </el-button>
      </div>
    </div>

    <div v-if="!loading && !sources.length" class="empty-panel card-surface">
      <span class="empty-icon"><el-icon><Box /></el-icon></span>
      <h3>尚未配置制品源</h3>
      <p>连接云效流水线后，可直接拉取最近一次成功构建，校验 MD5 并解包为可部署的交付包。</p>
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
      <span class="empty-hint">令牌保存在本机应用数据目录，仅当前用户可读</span>
    </div>

    <div v-else class="card-surface">
      <div class="card-title-bar">
        <h3>制品源</h3>
        <span class="hint">令牌保存在本机应用数据目录，仅当前用户可读</span>
      </div>
      <el-table :data="sources" size="small" class="source-table">
        <el-table-column label="名称" min-width="150">
          <template #default="{ row }">
            <div class="name-cell">
              <span>{{ row.name }}</span>
              <el-tag size="small" effect="plain">
                {{ row.type === 'url' ? '直接下载' : '云效流水线' }}
              </el-tag>
              <el-tag v-if="isInUse(row)" size="small" type="success" effect="plain">
                使用中
              </el-tag>
            </div>
          </template>
        </el-table-column>
        <el-table-column label="流水线" min-width="100">
          <template #default="{ row }">
            <el-tooltip
              v-if="row.type !== 'url'"
              :content="`组织 ${row.orgId} / 流水线 ${row.pipelineId}`"
              placement="top"
            >
              <span>#{{ row.pipelineId }}</span>
            </el-tooltip>
            <span v-else class="muted">直链</span>
          </template>
        </el-table-column>
        <el-table-column prop="artifactName" label="制品名" min-width="100" />
        <el-table-column label="最近版本" width="80">
          <template #default="{ row }">
            <span>{{ row.lastVersion || '-' }}</span>
          </template>
        </el-table-column>
        <el-table-column label="本地目录" min-width="130">
          <template #default="{ row }">
            <el-tooltip
              v-if="row.lastBundleDir"
              :content="row.lastBundleDir"
              placement="top"
            >
              <code class="path-text">{{ shortPath(row.lastBundleDir) }}</code>
            </el-tooltip>
            <span v-else class="muted">-</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="340">
          <template #default="{ row }">
            <el-button link type="primary" :loading="loadingId === row.id" @click="checkLatest(row)">
              <span>检查最新</span>
            </el-button>
            <el-button v-if="row.lastBundleDir" link type="primary" @click="useExisting(row)">
              <span>使用已下载</span>
            </el-button>
            <el-button
              v-if="row.lastBundleDir"
              link
              type="warning"
              :loading="removingId === row.id"
              @click="removeLocal(row)"
            >
              删除本地包
            </el-button>
            <el-button link @click="openEdit(row)">编辑</el-button>
            <el-button link type="danger" @click="remove(row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
    </div>

    <div v-if="bundleDirText" class="plan-section">
      <div v-if="planLoading" v-loading="true" class="plan-loading card-surface">
        <span>正在读取交付包内容</span>
      </div>

      <el-alert
        v-else-if="planError"
        title="当前交付包目录不可读"
        :description="planError"
        type="warning"
        :closable="false"
        show-icon
      />

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

    <ArtifactSourceEditor
      v-model="editorVisible"
      :source="editing"
      @saved="loadSources"
    />

    <el-dialog v-model="releaseVisible" title="最新构建" width="640px" :close-on-click-modal="!downloading">
      <div v-if="release" class="release-body">
        <div class="release-grid">
          <span class="label">运行编号</span>
          <span class="value">#{{ release.runId }}</span>
          <span class="label">版本</span>
          <span class="value">{{ release.version || '-' }}</span>
          <span class="label">提交</span>
          <span class="value">{{ release.commitShort || '-' }} {{ release.commitTitle }}</span>
          <span class="label">分支</span>
          <span class="value">{{ release.branch || '-' }}</span>
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
          :description="`版本 ${bundle.version || '-'}，共 ${bundle.files} 个文件，MD5 校验通过`"
          type="success"
          :closable="false"
          show-icon
        />
      </div>

      <template #footer>
        <el-button v-if="release && release.runUrl" link type="primary" @click="openRunUrl">
          打开运行详情
        </el-button>
        <el-button @click="releaseVisible = false">关闭</el-button>
        <el-button
          v-if="bundle"
          type="primary"
          @click="applyBundle"
        >
          设为当前部署包
        </el-button>
        <el-button
          v-else
          type="primary"
          :loading="downloading"
          @click="download"
        >
          下载并解包
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref, watch } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { Box, Plus, Refresh } from '@element-plus/icons-vue';
import BundlePlanPanel from '../components/BundlePlanPanel.vue';
import ArtifactSourceEditor from '../components/ArtifactSourceEditor.vue';
import {
  deleteArtifactSource,
  deleteLocalBundle,
  downloadArtifact,
  fetchArtifactRelease,
  listArtifactSources,
  openArtifactBundle
} from '../api/artifact.js';
import { inspectBundlePlan } from '../api/tauri.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

defineEmits(['go-wizard', 'go-update']);

const sources = ref([]);
const loading = ref(false);
const loadingId = ref('');
const removingId = ref('');
const editorVisible = ref(false);
const editing = ref(null);
const releaseVisible = ref(false);
const release = ref(null);
const releaseSource = ref(null);
const downloading = ref(false);
const bundle = ref(null);
const progress = reactive({ percent: 0, stage: '' });
const plan = ref(null);
const planLoading = ref(false);
const planError = ref('');
const imageDir = ref(localStorage.getItem('image-local-dir') || '');

const bundleDirText = computed(() => String(props.connection.bundleDir || '').trim());

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
    planError.value = '';
    return;
  }
  planLoading.value = true;
  planError.value = '';
  try {
    plan.value = await inspectBundlePlan(dir, remoteTarget.value);
  } catch (error) {
    plan.value = null;
    planError.value = `读取交付包失败：${error}`;
  } finally {
    planLoading.value = false;
  }
}

watch(bundleDirText, loadPlan, { immediate: true });

async function loadSources() {
  loading.value = true;
  try {
    sources.value = await listArtifactSources();
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    loading.value = false;
  }
}

function openCreate() {
  editing.value = null;
  editorVisible.value = true;
}

function openEdit(row) {
  editing.value = row;
  editorVisible.value = true;
}

function isInUse(row) {
  const current = String(props.connection.bundleDir || '').trim();
  const dir = String(row.lastBundleDir || '').trim();
  if (!current || !dir) return false;
  return current === dir || current.startsWith(`${dir}/`);
}

async function removeLocal(row) {
  const inUse = isInUse(row);
  const message = inUse
    ? `确认删除该制品源的本地交付包？\n\n${row.lastBundleDir}\n\n该包当前正在使用，删除后需要重新下载并选择交付包。`
    : `确认删除该制品源的本地交付包？\n\n${row.lastBundleDir}\n\n本地缓存的全部版本都会被删除，云端制品不受影响。`;
  try {
    await ElMessageBox.confirm(message, '删除本地包', {
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
    ElMessage.success(`已删除本地交付包，释放 ${formatSize(result.freedBytes)}`);
    await loadSources();
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

async function checkLatest(row) {
  loadingId.value = row.id;
  try {
    const info = await fetchArtifactRelease(row.id, null);
    releaseSource.value = row;
    release.value = info;
    bundle.value = null;
    progress.percent = 0;
    progress.stage = '';
    releaseVisible.value = true;
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    loadingId.value = '';
  }
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
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    downloading.value = false;
  }
}

async function useExisting(row) {
  loadingId.value = row.id;
  try {
    const result = await openArtifactBundle(row.id);
    props.connection.bundleDir = result.dir;
    ElMessage.success(`已使用本地交付包 ${result.version || ''}`.trim());
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    loadingId.value = '';
  }
}

function applyBundle() {
  if (!bundle.value) return;
  props.connection.bundleDir = bundle.value.dir;
  ElMessage.success('已设为当前部署包');
  releaseVisible.value = false;
}

async function openRunUrl() {
  const url = release.value?.runUrl;
  if (!url) return;
  if (!isTauri()) {
    window.open(url, '_blank', 'noopener');
    return;
  }
  try {
    await invoke('open_external_url', { url });
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

.source-table {
  margin-top: 8px;
}

.source-table :deep(.el-button + .el-button) {
  margin-left: 8px;
}

.source-table :deep(.cell) {
  white-space: nowrap;
}

.name-cell {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.name-cell span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
</style>
