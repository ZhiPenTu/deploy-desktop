<template>
  <div class="page-view settings-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">设置</h2>
        <p class="page-subtitle">按板块管理制品来源、镜像拉取与系统配置</p>
      </div>
    </div>

    <el-tabs v-model="activeSection" class="settings-tabs">
      <el-tab-pane label="制品设置" name="artifacts">
        <div class="card-surface settings-card">
          <div class="card-title-bar">
            <h3>访问令牌</h3>
            <span class="hint">令牌保存在本机应用数据目录，仅当前用户可读</span>
          </div>

          <el-table v-if="tokens.length" :data="tokens" size="small" class="source-table">
            <el-table-column label="名称" min-width="180">
              <template #default="{ row }">
                <div class="name-cell">
                  <span>{{ row.name }}</span>
                  <el-tag v-if="!row.hasToken" size="small" type="warning" effect="plain">
                    内容缺失
                  </el-tag>
                </div>
              </template>
            </el-table-column>
            <el-table-column label="关联制品源" width="120">
              <template #default="{ row }">
                <span :class="row.inUse ? '' : 'muted-text'">
                  {{ row.inUse ? `${row.inUse} 个` : '未关联' }}
                </span>
              </template>
            </el-table-column>
            <el-table-column label="更新于" width="150">
              <template #default="{ row }">{{ formatTime(row.updatedAt) }}</template>
            </el-table-column>
            <el-table-column label="操作" width="140">
              <template #default="{ row }">
                <el-button link type="primary" @click="openTokenEdit(row)">编辑</el-button>
                <el-button link type="danger" :disabled="row.inUse > 0" @click="removeToken(row)">
                  删除
                </el-button>
              </template>
            </el-table-column>
          </el-table>

          <div v-else class="empty-state">
            <span class="icon-tile neutral"><el-icon><Key /></el-icon></span>
            <span class="empty-title">还没有保存的访问令牌</span>
            <span class="empty-hint">保存云效个人访问令牌后，多个制品源可以直接选用，不必重复填写。</span>
          </div>

          <div class="card-actions">
            <el-button type="primary" @click="openTokenCreate">
              <el-icon><Plus /></el-icon>
              <span>新建令牌</span>
            </el-button>
            <el-button :loading="tokenLoading" @click="loadTokens">
              <el-icon><Refresh /></el-icon>
              <span>刷新</span>
            </el-button>
          </div>
        </div>

        <div class="card-surface settings-card">
          <div class="card-title-bar">
            <h3>云效制品源</h3>
            <span class="hint">令牌保存在本机应用数据目录，仅当前用户可读</span>
          </div>

          <el-table v-if="sources.length" :data="sources" size="small" class="source-table">
            <el-table-column label="名称" min-width="160">
              <template #default="{ row }">
                <div class="name-cell">
                  <span>{{ row.name }}</span>
                  <el-tag size="small" effect="plain">
                    {{ row.type === 'url' ? '直接下载' : '云效流水线' }}
                  </el-tag>
                </div>
              </template>
            </el-table-column>
            <el-table-column label="组织 / 流水线" min-width="180">
              <template #default="{ row }">
                <span v-if="row.type !== 'url'" class="mono-text">{{ row.orgId }} / #{{ row.pipelineId }}</span>
                <span v-else class="muted-text">直链下载</span>
              </template>
            </el-table-column>
            <el-table-column prop="artifactName" label="制品名" min-width="130" />
            <el-table-column label="访问令牌" width="140">
              <template #default="{ row }">
                <span v-if="row.type === 'url'" class="muted-text">不适用</span>
                <span v-else-if="tokenName(row.tokenId)" class="muted-text">
                  {{ tokenName(row.tokenId) }}
                </span>
                <el-tag v-else :type="row.hasToken ? 'success' : 'warning'" size="small" effect="plain">
                  {{ row.hasToken ? '已保存' : '未配置' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="操作" width="150">
              <template #default="{ row }">
                <el-button link type="primary" @click="openEdit(row)">编辑</el-button>
                <el-button link type="danger" @click="remove(row)">删除</el-button>
              </template>
            </el-table-column>
          </el-table>

          <div v-else class="empty-state">
            <span class="icon-tile neutral"><el-icon><Box /></el-icon></span>
            <span class="empty-title">还没有配置云效制品源</span>
            <span class="empty-hint">填写云效组织 ID、流水线 ID 并选择访问令牌后，即可在制品中心拉取离线交付包。</span>
          </div>

          <div class="card-actions">
            <el-button type="primary" @click="openCreate">
              <el-icon><Plus /></el-icon>
              <span>新建制品源</span>
            </el-button>
            <el-button :loading="loading" @click="loadSources">
              <el-icon><Refresh /></el-icon>
              <span>刷新</span>
            </el-button>
            <el-button @click="$emit('go-artifacts')">前往制品中心</el-button>
          </div>
        </div>
      </el-tab-pane>

      <el-tab-pane label="镜像设置" name="images" lazy>
        <ManifestPullPanel :connection="connection" />

        <div class="card-surface settings-card">
          <div class="card-title-bar">
            <h3>离线镜像包</h3>
            <span class="hint">从镜像列表导出的 tar 包保存在此目录</span>
          </div>
          <el-form label-width="110px">
            <el-form-item label="保存目录">
              <div class="dir-row">
                <el-input v-model="offlineDir" placeholder="/path/to/offline-images" />
                <el-button @click="pickOfflineDir">
                  <el-icon><FolderOpened /></el-icon>
                  <span>选择目录</span>
                </el-button>
              </div>
              <div class="field-hint">
                默认目录：{{ defaultOfflineDir || '读取中' }}。导出、导入和删除离线包都以该目录为准。
              </div>
            </el-form-item>
          </el-form>
          <div class="card-actions">
            <el-button type="primary" :loading="savingOfflineDir" @click="saveOfflineDir">保存目录</el-button>
            <el-button :disabled="!defaultOfflineDir || savingOfflineDir" @click="resetOfflineDir">
              恢复默认目录
            </el-button>
            <el-button @click="$emit('go-images')">前往镜像与离线包</el-button>
          </div>
        </div>
      </el-tab-pane>

      <el-tab-pane label="系统设置" name="system">
        <div class="card-surface settings-card">
          <div class="card-title-bar">
            <h3>软件更新</h3>
            <span class="hint">直接从 GitHub Releases 获取最新版本</span>
          </div>
          <div class="info-grid">
            <span class="info-label">当前版本</span>
            <span class="info-value">v{{ appVersion }}</span>
            <span class="info-label">更新来源</span>
            <span class="info-value">GitHub Releases</span>
          </div>
          <p class="field-hint">
            检查与下载均在本机完成，无需配置更新源。发现新版本后可直接安装，应用会自动重启。
          </p>
          <div class="card-actions">
            <UpdateChecker />
          </div>
        </div>

        <div class="card-surface settings-card">
          <div class="card-title-bar">
            <h3>当前部署目标</h3>
            <span class="hint">随当前项目变化</span>
          </div>
          <div class="info-grid">
            <span class="info-label">当前项目</span>
            <span class="info-value">{{ project?.name || '未选择项目' }}</span>
            <span class="info-label">部署方式</span>
            <span class="info-value">{{ isRemote ? '远程局域网部署' : '本机离线部署' }}</span>
            <span class="info-label">目标主机</span>
            <span class="info-value">{{ isRemote ? connection.remoteHost || '未填写' : '当前工作站' }}</span>
            <span class="info-label">部署目录</span>
            <span class="info-value path-value">{{ connection.installDir || '未设置' }}</span>
            <span class="info-label">交付包位置</span>
            <span class="info-value path-value">{{ connection.bundleDir || '未设置' }}</span>
            <span class="info-label">交付包版本</span>
            <span class="info-value">{{ project?.lastVersion || '未绑定' }}</span>
          </div>
          <p class="field-hint">
            项目在左侧「项目管理」中维护，交付包在制品中心选择；在部署向导中选择项目后，监控、日志与数据库都会使用该项目的主机与目录。
          </p>
        </div>
      </el-tab-pane>

      <el-tab-pane label="运行日志" name="logs" lazy>
        <RuntimeLogs />
      </el-tab-pane>
    </el-tabs>

    <ArtifactSourceEditor
      v-model="editorVisible"
      :source="editing"
      @saved="reloadSources"
    />

    <ArtifactTokenEditor
      v-model="tokenEditorVisible"
      :token="editingToken"
      @saved="loadTokens"
    />
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Box, FolderOpened, Key, Plus, Refresh } from '@element-plus/icons-vue';
import { selectLocalPath as open } from '../../../api/tauri.js';
import ArtifactSourceEditor from '../../artifacts/components/ArtifactSourceEditor.vue';
import ArtifactTokenEditor from '../../artifacts/components/ArtifactTokenEditor.vue';
import ManifestPullPanel from '../../registry/components/ManifestPullPanel.vue';
import UpdateChecker from '../components/UpdateChecker.vue';
import RuntimeLogs from '../components/RuntimeLogs.vue';
import { projectState } from '../../../config/project.js';
import {
  deleteArtifactSource,
  deleteArtifactToken,
  listArtifactSources,
  listArtifactTokens,
  getImageLibrarySettings,
  saveImageLibrarySettings
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  },
  initialSection: {
    type: String,
    default: 'artifacts'
  }
});

defineEmits(['go-artifacts', 'go-images']);

const appVersion = __APP_VERSION__;
const activeSection = ref(props.initialSection || 'artifacts');
const sources = ref([]);
const loading = ref(false);
const editorVisible = ref(false);
const editing = ref(null);
const tokens = ref([]);
const tokenLoading = ref(false);
const tokenEditorVisible = ref(false);
const editingToken = ref(null);
const offlineDir = ref('');
const defaultOfflineDir = ref('');
const savingOfflineDir = ref(false);

const isRemote = computed(() => props.connection.mode === 'remote');
const project = computed(() => projectState.project);

async function loadSources() {
  loading.value = true;
  try {
    sources.value = await listArtifactSources();
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    loading.value = false;
  }
}

async function loadTokens() {
  tokenLoading.value = true;
  try {
    tokens.value = await listArtifactTokens();
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    tokenLoading.value = false;
  }
}

function reloadSources() {
  loadSources();
  loadTokens();
}

function tokenName(tokenId) {
  if (!tokenId) return '';
  const match = tokens.value.find((item) => item.id === tokenId);
  return match ? match.name : '';
}

function formatTime(value) {
  if (!value) return '未记录';
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  }).format(new Date(value));
}

function openTokenCreate() {
  editingToken.value = null;
  tokenEditorVisible.value = true;
}

function openTokenEdit(row) {
  editingToken.value = row;
  tokenEditorVisible.value = true;
}

async function removeToken(row) {
  try {
    await ElMessageBox.confirm(
      `确认删除访问令牌「${row.name}」？\n\n此操作无法撤销。`,
      '删除访问令牌',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' }
    );
  } catch {
    return;
  }
  try {
    await deleteArtifactToken(row.id);
    ElMessage.success('访问令牌已删除');
    await reloadSources();
  } catch (error) {
    ElMessage.error(errorText(error));
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

async function loadImageSettings() {
  try {
    const settings = await getImageLibrarySettings();
    offlineDir.value = settings.offlineDir || '';
    defaultOfflineDir.value = settings.defaultDir || '';
  } catch (error) {
    ElMessage.error(errorText(error));
  }
}

async function pickOfflineDir() {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择离线镜像包保存目录'
    });
    if (typeof selected === 'string' && selected) {
      offlineDir.value = selected;
    }
  } catch (error) {
    ElMessage.error(`选择目录失败：${errorText(error)}`);
  }
}

async function resetOfflineDir() {
  if (!defaultOfflineDir.value) {
    return;
  }
  offlineDir.value = defaultOfflineDir.value;
  await saveOfflineDir();
}

async function saveOfflineDir() {
  const dir = offlineDir.value.trim();
  if (!dir) {
    ElMessage.warning('请填写离线包保存目录');
    return;
  }
  savingOfflineDir.value = true;
  try {
    const settings = await saveImageLibrarySettings(dir);
    offlineDir.value = settings.offlineDir || dir;
    defaultOfflineDir.value = settings.defaultDir || defaultOfflineDir.value;
    ElMessage.success('离线包保存目录已保存');
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    savingOfflineDir.value = false;
  }
}

async function remove(row) {
  try {
    await ElMessageBox.confirm(
      `确认删除制品源「${row.name}」？\n\n已下载的本地交付包不会被删除，访问令牌仍保留在令牌列表中。`,
      '删除制品源',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' }
    );
  } catch {
    return;
  }
  try {
    await deleteArtifactSource(row.id);
    ElMessage.success('制品源已删除');
    await loadSources();
  } catch (error) {
    ElMessage.error(errorText(error));
  }
}

onMounted(() => {
  loadSources();
  loadTokens();
  loadImageSettings();
});
</script>

<style scoped>
.settings-tabs {
  height: 100%;
}

.settings-tabs :deep(.el-tabs__header) {
  margin-bottom: 18px;
}

.settings-tabs :deep(.el-tabs__content) {
  overflow: visible;
}

.settings-card {
  padding: 20px;
  margin-bottom: 18px;
}

.card-title-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 14px;
  margin-bottom: 16px;
}

.card-title-bar h3 {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-main);
}

.card-title-bar .hint {
  font-size: 11px;
  color: var(--text-muted);
}

.name-cell {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.source-table {
  width: 100%;
  background: transparent;
}

.mono-text {
  font-size: 11px;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  color: var(--text-secondary);
}

.muted-text {
  color: var(--text-muted);
  font-size: 12px;
}

.card-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-top: 16px;
}

.info-grid {
  display: grid;
  grid-template-columns: 110px minmax(0, 1fr);
  gap: 10px 14px;
  font-size: 12px;
}

.dir-row {
  display: flex;
  gap: 10px;
  width: 100%;
}

.info-label {
  color: var(--text-muted);
}

.info-value {
  color: var(--text-main);
}

.path-value {
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  font-size: 11px;
  overflow-wrap: anywhere;
}

.field-hint {
  margin: 12px 0 0;
  font-size: 11px;
  line-height: 1.6;
  color: var(--text-muted);
}

</style>
