<template>
  <div class="page-view registry-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">镜像仓库</h2>
        <p class="page-subtitle">可选高级能力：按清单从镜像仓库拉取镜像；标准交付在制品中心与“镜像与离线包”页面完成</p>
      </div>
      <div class="header-actions">
        <el-tag v-if="isRemote" type="primary" effect="plain">
          远程 {{ connection.remoteHost || '未配置主机' }}
        </el-tag>
        <el-tag v-else type="info" effect="plain">本地部署</el-tag>
        <el-button :loading="loading" @click="loadManifest">
          <el-icon><DocumentChecked /></el-icon>
          <span>加载清单</span>
        </el-button>
      </div>
    </div>

    <el-alert
      :title="isRemote ? '远程模式在目标机执行 docker login 与 docker pull' : '本地模式在当前机器执行 docker login 与 docker pull'"
      description="标准离线交付无需配置镜像仓库。仅在需要从远端仓库补充镜像时使用；凭据只保存在本次运行内存中，清单必须用 sha256 摘要固定版本。"
      type="info"
      :closable="false"
      show-icon
      class="mode-alert"
    />

    <div class="card-surface registry-card">
      <div class="card-title-bar">
        <h3>镜像清单</h3>
        <span class="hint">支持 HTTP(S) 地址、本地 YAML 或 JSON 文件，最大 5 MB</span>
      </div>

      <el-form label-width="110px">
        <el-form-item label="清单来源">
          <div class="source-row">
            <el-input
              v-model="manifestSource"
              placeholder="https://example.com/manifest.yaml 或 /path/to/manifest.yaml"
              clearable
              @keyup.enter="loadManifest"
            />
            <el-button @click="pickManifestFile">选择文件</el-button>
          </div>
        </el-form-item>
      </el-form>

      <div v-if="summary" class="summary-block">
        <div class="summary-grid">
          <div class="summary-item">
            <span class="summary-label">产品</span>
            <span class="summary-value">{{ summary.manifest.metadata.product || summary.manifest.metadata.name }}</span>
          </div>
          <div class="summary-item">
            <span class="summary-label">版本</span>
            <span class="summary-value">{{ summary.manifest.metadata.version }}</span>
          </div>
          <div class="summary-item">
            <span class="summary-label">仓库</span>
            <span class="summary-value">{{ registryPath }}</span>
          </div>
          <div class="summary-item">
            <span class="summary-label">平台</span>
            <span class="summary-value">{{ summary.manifest.spec.platform || '按目标机架构' }}</span>
          </div>
          <div class="summary-item">
            <span class="summary-label">镜像数量</span>
            <span class="summary-value">{{ summary.images.length }} 个</span>
          </div>
          <div class="summary-item">
            <span class="summary-label">预计体积</span>
            <span class="summary-value">{{ formatSize(summary.total_size_bytes) }}</span>
          </div>
        </div>

        <div v-if="summary.warnings.length" class="warning-list">
          <span v-for="warning in summary.warnings" :key="warning" class="warning-chip">{{ warning }}</span>
        </div>

        <el-table
          ref="imageTable"
          :data="summary.images"
          row-key="name"
          size="small"
          class="image-table"
          @selection-change="handleSelectionChange"
        >
          <el-table-column type="selection" width="44" />
          <el-table-column prop="name" label="镜像名称" width="150" />
          <el-table-column label="镜像地址" min-width="320">
            <template #default="{ row }">
              <code>{{ row.reference }}</code>
            </template>
          </el-table-column>
          <el-table-column label="平台" width="130">
            <template #default="{ row }">{{ row.platform || '默认' }}</template>
          </el-table-column>
          <el-table-column label="大小" width="110">
            <template #default="{ row }">
              {{ row.size_bytes ? formatSize(row.size_bytes) : '未提供' }}
            </template>
          </el-table-column>
        </el-table>
      </div>

      <div v-else class="empty-state">
        {{ loading ? '正在加载清单' : '填写清单地址后点击“加载清单”预览镜像' }}
      </div>
    </div>

    <div class="card-surface registry-card">
      <div class="card-title-bar">
        <h3>仓库凭据与离线导出</h3>
        <span class="hint">凭据可留空，适用于公开仓库或目标机已登录的仓库</span>
      </div>

      <el-form label-width="110px">
        <el-form-item label="仓库用户名">
          <el-input v-model="registryUser" placeholder="阿里云 ACR 用户名" autocomplete="off" clearable />
        </el-form-item>
        <el-form-item label="仓库密码">
          <el-input
            v-model="registryPassword"
            type="password"
            show-password
            placeholder="仅保存在当前运行内存中"
            autocomplete="new-password"
          />
        </el-form-item>
        <el-form-item label="导出目录">
          <div class="source-row">
            <el-input
              v-model="exportDir"
              :placeholder="isRemote ? '目标机路径，例如 /opt/deploy/images（可选）' : '本机路径，例如 ~/deploy-images（可选）'"
              clearable
            />
            <el-button v-if="!isRemote" @click="pickExportDir">选择目录</el-button>
          </div>
          <div class="field-hint">
            填写导出目录后，每个镜像会在拉取完成后执行 docker save 生成 tar 包；留空则只拉取到 Docker。
          </div>
        </el-form-item>
      </el-form>

      <div class="action-row">
        <span class="selection-hint">已选择 {{ selectedNames.length }} / {{ summary ? summary.images.length : 0 }} 个镜像</span>
        <el-button
          type="primary"
          :loading="pulling"
          :disabled="!canPull"
          @click="startPull"
        >
          <el-icon><Download /></el-icon>
          <span>{{ isRemote ? '拉取到目标机' : '拉取到本机' }}</span>
        </el-button>
      </div>
    </div>

    <div v-if="pulling || progress.length" class="card-surface registry-card">
      <div class="card-title-bar">
        <h3>拉取进度</h3>
        <span class="hint">{{ progressMessage }}</span>
      </div>
      <el-progress
        :percentage="currentPercent"
        :status="currentPercent >= 100 ? 'success' : undefined"
        class="progress-bar"
      />
      <div class="progress-list">
        <div v-for="(item, index) in progress" :key="`${index}-${item.message}`" class="progress-row">
          <span class="progress-percent">{{ item.percent }}%</span>
          <span class="progress-text">{{ item.message }}</span>
        </div>
      </div>
    </div>

    <div v-if="results.length || resultLog" class="card-surface registry-card">
      <div class="card-title-bar">
        <h3>执行结果</h3>
        <span v-if="results.length" class="hint">共 {{ results.length }} 个镜像</span>
      </div>

      <el-table v-if="results.length" :data="results" size="small" class="image-table">
        <el-table-column prop="name" label="镜像名称" width="150" />
        <el-table-column label="状态" width="120">
          <template #default="{ row }">
            <el-tag type="success" size="small">{{ row.message }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="导出文件" min-width="260">
          <template #default="{ row }">
            <code v-if="row.exported_path">{{ row.exported_path }}</code>
            <span v-else class="muted-text">未导出</span>
          </template>
        </el-table-column>
        <el-table-column label="大小" width="110">
          <template #default="{ row }">{{ formatSize(row.size_bytes) }}</template>
        </el-table-column>
      </el-table>

      <pre v-if="resultLog" class="output-log">{{ resultLog }}</pre>
    </div>
  </div>
</template>

<script setup>
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { ElMessage } from 'element-plus';
import { DocumentChecked, Download } from '@element-plus/icons-vue';
import { open } from '@tauri-apps/plugin-dialog';
import {
  loadImageManifest,
  onRegistryProgress,
  pullManifestImagesLocal,
  pullManifestImagesRemote
} from '../api/tauri.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const isRemote = computed(() => props.connection.mode === 'remote');
const manifestSource = ref('');
const summary = ref(null);
const imageTable = ref(null);
const selectedNames = ref([]);
const registryUser = ref('');
const registryPassword = ref('');
const exportDir = ref('');
const progress = ref([]);
const results = ref([]);
const resultLog = ref('');
const loading = ref(false);
const pulling = ref(false);
let unlistenProgress = null;

const registryPath = computed(() => {
  if (!summary.value) {
    return '';
  }
  const { registry, namespace } = summary.value.manifest.spec;
  return namespace ? `${registry}/${namespace}` : registry;
});

const currentPercent = computed(() => {
  if (!progress.value.length) {
    return 0;
  }
  return progress.value[progress.value.length - 1].percent;
});

const progressMessage = computed(() => {
  if (!progress.value.length) {
    return pulling.value ? '准备中' : '';
  }
  const last = progress.value[progress.value.length - 1];
  return `${last.current} / ${last.total} · ${last.image}`;
});

const canPull = computed(
  () => Boolean(summary.value) && selectedNames.value.length > 0 && !pulling.value && !loading.value
);

const formatSize = (bytes) => {
  const value = Number(bytes) || 0;
  if (value >= 1024 * 1024 * 1024) {
    return `${(value / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }
  if (value >= 1024 * 1024) {
    return `${(value / 1024 / 1024).toFixed(1)} MB`;
  }
  if (value >= 1024) {
    return `${(value / 1024).toFixed(0)} KB`;
  }
  return `${value} B`;
};

const handleSelectionChange = (rows) => {
  selectedNames.value = rows.map((row) => row.name);
};

const pickManifestFile = async () => {
  try {
    const selected = await open({
      multiple: false,
      filters: [{ name: '镜像清单', extensions: ['yaml', 'yml', 'json'] }]
    });
    if (typeof selected === 'string' && selected) {
      manifestSource.value = selected;
    }
  } catch (e) {
    ElMessage.error(`选择清单文件失败: ${e}`);
  }
};

const pickExportDir = async () => {
  try {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === 'string' && selected) {
      exportDir.value = selected;
    }
  } catch (e) {
    ElMessage.error(`选择导出目录失败: ${e}`);
  }
};

const loadManifest = async () => {
  const source = manifestSource.value.trim();
  if (!source) {
    ElMessage.warning('请填写清单地址或选择本地文件');
    return;
  }
  loading.value = true;
  summary.value = null;
  results.value = [];
  resultLog.value = '';
  progress.value = [];
  selectedNames.value = [];
  try {
    summary.value = await loadImageManifest(source);
    manifestSource.value = summary.value.source;
    await nextTick();
    const table = imageTable.value;
    if (table) {
      summary.value.images.forEach((row) => table.toggleRowSelection(row, true));
    }
    ElMessage.success(`清单已加载，共 ${summary.value.images.length} 个镜像`);
  } catch (e) {
    ElMessage.error(String(e));
  } finally {
    loading.value = false;
  }
};

const validateCredentials = () => {
  if (registryPassword.value && !registryUser.value.trim()) {
    ElMessage.warning('已填写仓库密码，请同时填写仓库用户名');
    return false;
  }
  if (isRemote.value && !props.connection.remoteHost) {
    ElMessage.warning('请先在连接配置中填写目标主机');
    return false;
  }
  return true;
};

const startPull = async () => {
  if (!summary.value) {
    ElMessage.warning('请先加载镜像清单');
    return;
  }
  if (!selectedNames.value.length) {
    ElMessage.warning('请至少选择一个镜像');
    return;
  }
  if (!validateCredentials()) {
    return;
  }

  pulling.value = true;
  progress.value = [];
  results.value = [];
  resultLog.value = '';
  const username = registryUser.value.trim() || null;
  const password = registryPassword.value ? registryPassword.value : null;
  const targetExportDir = exportDir.value.trim() || null;

  try {
    if (isRemote.value) {
      results.value = await pullManifestImagesRemote(
        props.connection.remoteHost,
        props.connection.remotePort,
        props.connection.remoteUser,
        props.connection.remoteKey || null,
        summary.value.source,
        username,
        password,
        selectedNames.value,
        targetExportDir
      );
    } else {
      results.value = await pullManifestImagesLocal(
        summary.value.source,
        username,
        password,
        selectedNames.value,
        targetExportDir
      );
    }
    resultLog.value = results.value
      .map((item) => `${item.name}  ${item.reference}  ${item.message}`)
      .join('\n');
    ElMessage.success(`已完成 ${results.value.length} 个镜像的拉取`);
  } catch (e) {
    resultLog.value = String(e);
    ElMessage.error('镜像拉取失败，请查看执行结果');
  } finally {
    pulling.value = false;
  }
};

onMounted(async () => {
  unlistenProgress = await onRegistryProgress((payload) => {
    progress.value.push(payload);
  });
  if (props.connection.mode === 'local') {
    exportDir.value = '';
  }
});

onUnmounted(() => {
  if (typeof unlistenProgress === 'function') {
    unlistenProgress();
  }
});
</script>

<style scoped>
.registry-card {
  padding: 20px;
  margin-bottom: 18px;
}

.mode-alert {
  margin-bottom: 20px;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
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

.source-row {
  display: flex;
  gap: 10px;
  width: 100%;
}

.field-hint {
  width: 100%;
  margin-top: 6px;
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-muted);
}

.summary-block {
  margin-top: 4px;
}

.summary-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 12px;
  margin-bottom: 14px;
}

.summary-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 12px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-color);
  background: var(--panel-subtle);
}

.summary-label {
  font-size: 10px;
  color: var(--text-muted);
  letter-spacing: 0.04em;
}

.summary-value {
  font-size: 12px;
  color: var(--text-secondary);
  overflow-wrap: anywhere;
}

.warning-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 14px;
}

.warning-chip {
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 999px;
  color: var(--warning-color);
  background: rgba(242, 196, 109, 0.1);
  border: 1px solid rgba(242, 196, 109, 0.22);
}

.image-table {
  width: 100%;
  background: transparent;
}

.image-table :deep(code) {
  color: var(--primary-color);
  font-size: 11px;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  overflow-wrap: anywhere;
}

.muted-text {
  color: var(--text-muted);
  font-size: 12px;
}

.empty-state {
  min-height: 120px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  font-size: 12px;
  border: 1px dashed var(--border-color);
  border-radius: var(--radius-sm);
}

.action-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.selection-hint {
  font-size: 12px;
  color: var(--text-muted);
}

.progress-bar {
  margin-bottom: 12px;
}

.progress-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 200px;
  overflow: auto;
}

.progress-row {
  display: flex;
  gap: 10px;
  align-items: baseline;
  font-size: 11px;
  color: var(--text-muted);
}

.progress-percent {
  width: 42px;
  text-align: right;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  color: var(--text-secondary);
}

.progress-text {
  overflow-wrap: anywhere;
}

.output-log {
  max-height: 240px;
  overflow: auto;
  margin: 0;
  padding: 12px;
  border-radius: var(--radius-sm);
  background: #0b0f17;
  border: 1px solid var(--border-color);
  color: #b9c6d8;
  font-size: 11px;
  line-height: 1.55;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: pre-wrap;
}
</style>
