<template>
  <div class="manifest-pull-panel">
    <el-alert
      :title="isRemote ? '镜像将拉取到当前目标机' : '镜像将拉取到本机 Docker'"
      description="可选择已保存的镜像源，或使用清单中的仓库地址。请确认镜像地址与拉取目标，镜像版本使用 sha256 摘要固定。"
      type="info"
      :closable="false"
      show-icon
      class="mode-alert"
    />

    <RegistrySourceManager :disabled="pulling || loading" @changed="updateRegistrySources" />

    <div class="card-surface registry-card">
      <div class="card-title-bar">
        <h3>镜像清单</h3>
        <span class="hint">支持 HTTP(S) 地址、本地 YAML 或 JSON 文件，最大 5 MB</span>
      </div>

      <el-form label-width="110px" :disabled="pulling || loading">
        <el-form-item label="镜像源">
          <el-select v-model="registrySourceId" clearable placeholder="使用清单中的仓库" style="width: 100%" @change="resetManifest">
            <el-option label="使用清单中的仓库" value="" />
            <el-option v-for="source in registrySources" :key="source.id" :label="`${source.name}（${source.address}）`" :value="source.id" />
          </el-select>
          <div class="field-hint">选择镜像源后将使用其仓库地址；已填写的命名空间会替换清单中的命名空间。请重新加载清单并确认镜像地址。</div>
        </el-form-item>
        <el-form-item label="清单来源">
          <div class="source-row">
            <el-input
              v-model="manifestSource"
              placeholder="https://example.com/manifest.yaml 或 /path/to/manifest.yaml"
              clearable
              @input="resetManifest"
              @keyup.enter="loadManifest"
            />
            <el-button @click="pickManifestFile">选择文件</el-button>
            <el-button type="primary" :loading="loading" :disabled="pulling" @click="loadManifest">
              <el-icon><DocumentChecked /></el-icon>
              <span>加载清单</span>
            </el-button>
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
        <span class="icon-tile neutral"><el-icon><DocumentChecked /></el-icon></span>
        <span class="empty-hint">{{ loading ? '正在加载清单' : '填写清单地址后点击“加载清单”预览镜像' }}</span>
      </div>
    </div>

    <div class="card-surface registry-card">
      <div class="card-title-bar">
        <h3>镜像访问与离线导出</h3>
        <span class="hint">未选择镜像源时，可填写仅供本次拉取使用的凭据</span>
      </div>

      <el-form label-width="110px" :disabled="pulling">
        <el-form-item v-if="selectedRegistrySource" label="访问方式">
          <span>{{ selectedRegistrySource.username ? `${selectedRegistrySource.username}（${selectedRegistrySource.hasPassword ? '使用已保存的密码' : '请填写本次密码'}）` : '无需登录' }}</span>
        </el-form-item>
        <el-form-item v-else label="仓库用户名">
          <el-input v-model="registryUser" placeholder="公开仓库可留空" autocomplete="off" clearable />
        </el-form-item>
        <el-form-item v-if="!selectedRegistrySource || (selectedRegistrySource.username && !selectedRegistrySource.hasPassword)" label="密码或令牌">
          <el-input
            v-model="registryPassword"
            type="password"
            show-password
            placeholder="仅用于本次拉取，不会保存"
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
            填写目录后，镜像拉取完成会自动导出为 tar 离线包；留空则只拉取到 Docker。
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
import { useDesktopProgress } from '../../../shared/composables/useDesktopProgress.js';
import { computed, nextTick, onMounted, ref } from 'vue';
import { ElMessage } from 'element-plus';
import { DocumentChecked, Download } from '@element-plus/icons-vue';
import { selectLocalPath as open } from '../../../api/tauri.js';
import RegistrySourceManager from './RegistrySourceManager.vue';
import {
  loadImageManifest,
  onRegistryProgress,
  pullManifestImagesLocal,
  pullManifestImagesRemote
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';

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
const registrySourceId = ref('');
const registrySources = ref([]);
const selectedRegistrySource = computed(() => registrySources.value.find((source) => source.id === registrySourceId.value));

function resetManifest() {
  summary.value = null;
  selectedNames.value = [];
  registryUser.value = '';
  registryPassword.value = '';
}

function updateRegistrySources(sources) {
  const selected = sources.find((source) => source.id === registrySourceId.value);
  if (JSON.stringify(selected) !== JSON.stringify(selectedRegistrySource.value)) resetManifest();
  registrySources.value = sources;
  if (!selected) registrySourceId.value = '';
}

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
      resetManifest();
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
  if (loading.value || pulling.value) return;
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
    summary.value = await loadImageManifest(source, registrySourceId.value || null);
    manifestSource.value = summary.value.source;
    await nextTick();
    const table = imageTable.value;
    if (table) {
      summary.value.images.forEach((row) => table.toggleRowSelection(row, true));
    }
    ElMessage.success(`清单已加载，共 ${summary.value.images.length} 个镜像`);
  } catch (e) {
    ElMessage.error(errorText(e));
  } finally {
    loading.value = false;
  }
};

const validateCredentials = () => {
  const source = selectedRegistrySource.value;
  if (!source && registryPassword.value && !registryUser.value.trim()) {
    ElMessage.warning('已填写仓库密码，请同时填写仓库用户名');
    return false;
  }
  if ((source?.username && !source.hasPassword && !registryPassword.value) || (!source && registryUser.value.trim() && !registryPassword.value)) {
    ElMessage.warning('请填写仓库密码或访问令牌');
    return false;
  }
  if (isRemote.value && !props.connection.remoteHost) {
    ElMessage.warning('请先在连接配置中填写目标主机');
    return false;
  }
  return true;
};

const startPull = async () => {
  if (pulling.value || loading.value) return;
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
  const username = selectedRegistrySource.value ? null : registryUser.value.trim() || null;
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
        targetExportDir,
        registrySourceId.value || null
      );
    } else {
      results.value = await pullManifestImagesLocal(
        summary.value.source,
        username,
        password,
        selectedNames.value,
        targetExportDir,
        registrySourceId.value || null
      );
    }
    resultLog.value = results.value
      .map((item) => `${item.name}  ${item.reference}  ${item.message}`)
      .join('\n');
    ElMessage.success(`已完成 ${results.value.length} 个镜像的拉取`);
  } catch (e) {
    resultLog.value = errorText(e);
    ElMessage.error('镜像拉取失败，请查看执行结果');
  } finally {
    registryPassword.value = '';
    pulling.value = false;
  }
};

useDesktopProgress(onRegistryProgress, (payload) => {
  progress.value.push(payload);
}, () => ElMessage.warning('进度暂不可用，请等待执行结果。'));

onMounted(async () => {
  if (props.connection.mode === 'local') {
    exportDir.value = '';
  }
});
</script>

<style scoped>
.registry-card {
  padding: 20px;
  margin-bottom: 18px;
}

.mode-alert {
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
  padding: 14px;
  border-radius: var(--radius-sm);
  background: #0b1120;
  border: 1px solid #1e293b;
  color: #cbd5e1;
  font-size: 12px;
  line-height: 1.6;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: pre-wrap;
}
</style>
