<template>
  <div class="card-surface settings-card">
    <div class="card-title-bar">
      <h3>项目</h3>
      <span class="hint">每个项目对应一台主机、一个部署目录和一个制品源</span>
    </div>

    <div v-if="projects.length" class="project-grid">
      <article v-for="row in projects" :key="row.id" class="project-card">
        <header class="project-head">
          <div class="project-ident">
            <h4 class="project-name">{{ row.name }}</h4>
            <div class="project-tags">
              <el-tag v-if="row.product" size="small" effect="plain">{{ row.product }}</el-tag>
            </div>
          </div>
          <span v-if="row.lastVersion" class="bundle-version" :title="`交付包版本 ${row.lastVersion}`">
            {{ row.lastVersion }}
          </span>
          <span v-else class="bundle-version muted-text">未绑定交付包</span>
        </header>

        <dl class="project-meta">
          <div class="meta-row">
            <dt>目标主机</dt>
            <dd>{{ hostText(row.hostId) }}</dd>
          </div>
          <div class="meta-row">
            <dt>部署目录</dt>
            <dd class="mono-text" :title="row.installDir">{{ row.installDir }}</dd>
          </div>
          <div class="meta-row">
            <dt>制品源</dt>
            <dd :class="sourceText(row.sourceId) ? '' : 'muted-text'">{{ sourceText(row.sourceId) || '未指定' }}</dd>
          </div>
        </dl>

        <footer class="project-foot">
          <el-button type="primary" size="small" :aria-label="`部署 ${row.name}`" @click="deploy(row)">
            部署
          </el-button>
          <el-button size="small" :aria-label="`监控 ${row.name}`" @click="inspect(row, 'dashboard')">
            <el-icon><Monitor /></el-icon>
            <span>监控</span>
          </el-button>
          <el-button size="small" :aria-label="`日志 ${row.name}`" @click="inspect(row, 'logs')">
            <el-icon><Document /></el-icon>
            <span>日志</span>
          </el-button>
          <span class="foot-spacer"></span>
          <el-button link type="primary" :aria-label="`编辑 ${row.name}`" @click="openEdit(row)">编辑</el-button>
          <el-button link type="danger" :aria-label="`删除 ${row.name}`" @click="remove(row)">删除</el-button>
        </footer>
      </article>
    </div>

    <div v-else class="empty-state">
      <span class="empty-title">还没有项目</span>
      <span class="empty-hint">
        新建项目后填写目标主机与部署目录，再在制品中心选择交付包，即可开始部署。
      </span>
    </div>

    <div class="card-actions">
      <el-button type="primary" @click="openCreate">
        <el-icon><Plus /></el-icon>
        <span>新建项目</span>
      </el-button>
      <el-button :loading="loading" @click="load">
        <el-icon><Refresh /></el-icon>
        <span>刷新</span>
      </el-button>
    </div>
  </div>

  <el-dialog
    v-model="editorVisible"
    :title="editingId ? '编辑项目' : '新建项目'"
    width="520px"
    :close-on-click-modal="false"
  >
    <el-form label-width="96px" @submit.prevent>
      <el-form-item label="项目名称">
        <el-input v-model="form.name" placeholder="例如：生产环境" maxlength="60" />
      </el-form-item>
      <el-form-item label="目标主机">
        <el-select v-model="form.hostId" placeholder="请选择主机" class="full-width">
          <el-option
            v-for="host in hosts"
            :key="host.id"
            :value="host.id"
            :label="host.kind === 'local' ? '本机' : `${host.name}（${host.address}）`"
          />
        </el-select>
      </el-form-item>
      <el-form-item label="部署目录">
        <el-input v-model="form.installDir" placeholder="例如：/opt/demo" />
      </el-form-item>
      <el-form-item label="制品源">
        <el-select v-model="form.sourceId" placeholder="可选" clearable class="full-width">
          <el-option v-for="source in sources" :key="source.id" :value="source.id" :label="source.name" />
        </el-select>
      </el-form-item>
    </el-form>
    <p class="field-hint">
      部署目录是目标主机上的绝对路径，填写安装根目录（例如 /opt/demo）。交付包内的文件按包内声明写入该目录及其子目录，Compose 清单通常落在其下的 deploy 子目录。
    </p>
    <template #footer>
      <el-button @click="editorVisible = false">取消</el-button>
      <el-button type="primary" :loading="saving" @click="save">保存</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Document, Monitor, Plus, Refresh } from '@element-plus/icons-vue';
import {
  createProject,
  deleteProject,
  listArtifactSources,
  updateProject
} from '../../../api/tauri.js';
import { projectState } from '../../../config/project.js';
import {
  connectionInventory,
  hostById,
  reloadProjectState
} from '../../connections/state.js';

const emit = defineEmits(['deploy', 'inspect']);
const projects = computed(() => projectState.projects);
const hosts = computed(() => connectionInventory.hosts);
const sources = ref([]);
const loading = ref(false);
const saving = ref(false);
const editorVisible = ref(false);
const editingId = ref('');
const form = reactive({ name: '', hostId: '', installDir: '', sourceId: '' });

function hostText(id) {
  const host = hostById(id);
  if (!host) return '主机未配置';
  return host.kind === 'local' ? '本机' : `${host.name}（${host.address}）`;
}

function sourceText(id) {
  if (!id) return '';
  return sources.value.find((source) => source.id === id)?.name || '';
}

async function load() {
  loading.value = true;
  try {
    const [list] = await Promise.all([
      listArtifactSources(),
      reloadProjectState()
    ]);
    sources.value = Array.isArray(list) ? list : [];
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    loading.value = false;
  }
}

function openCreate() {
  editingId.value = '';
  Object.assign(form, {
    name: '',
    hostId: hosts.value[0]?.id || '',
    installDir: '',
    sourceId: ''
  });
  editorVisible.value = true;
}

function openEdit(row) {
  editingId.value = row.id;
  Object.assign(form, {
    name: row.name,
    hostId: row.hostId,
    installDir: row.installDir,
    sourceId: row.sourceId || ''
  });
  editorVisible.value = true;
}

async function save() {
  if (!form.name.trim()) {
    ElMessage.warning('请填写项目名称');
    return;
  }
  if (!form.hostId) {
    ElMessage.warning('请选择目标主机');
    return;
  }
  if (!form.installDir.trim()) {
    ElMessage.warning('请填写部署目录');
    return;
  }
  saving.value = true;
  try {
    const input = {
      id: editingId.value,
      name: form.name.trim(),
      hostId: form.hostId,
      installDir: form.installDir.trim(),
      sourceId: form.sourceId || ''
    };
    if (editingId.value) {
      await updateProject(input);
    } else {
      await createProject(input);
    }
    await reloadProjectState();
    editorVisible.value = false;
    ElMessage.success(editingId.value ? '项目已更新' : '项目已创建');
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    saving.value = false;
  }
}

function deploy(row) {
  emit('deploy', row.id);
}

function inspect(row, tab) {
  emit('inspect', { id: row.id, tab });
}

async function remove(row) {
  try {
    await ElMessageBox.confirm(
      `确认删除项目「${row.name}」？\n\n只移除这条记录，已下载的交付包与目标机上的文件不会被删除。`,
      '删除项目',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' }
    );
  } catch {
    return;
  }
  try {
    await deleteProject(row.id);
    await reloadProjectState();
    ElMessage.success('项目已删除');
  } catch (error) {
    ElMessage.error(String(error));
  }
}

onMounted(load);
</script>

<style scoped>
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

.project-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(360px, 1fr));
  gap: 14px;
}

.project-card {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 16px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md, 12px);
  background: var(--surface-subtle, #fff);
  transition: border-color 0.16s ease, box-shadow 0.16s ease;
}

.project-card:hover {
  border-color: var(--brand-border, rgba(51, 92, 255, 0.4));
  box-shadow: 0 8px 20px rgba(15, 23, 42, 0.06);
}

.project-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.project-ident {
  min-width: 0;
}

.project-name {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-main);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.project-tags {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  margin-top: 6px;
}

.bundle-version {
  flex: none;
  max-width: 46%;
  font-size: 11px;
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.project-meta {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px;
  border-radius: var(--radius-sm);
  background: var(--surface-muted, rgba(15, 23, 42, 0.03));
}

.meta-row {
  display: grid;
  grid-template-columns: 68px minmax(0, 1fr);
  align-items: baseline;
  gap: 10px;
  font-size: 12px;
}

.meta-row dt {
  color: var(--text-muted);
}

.meta-row dd {
  min-width: 0;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meta-row dd.mono-text {
  font-size: 11px;
}

.project-foot {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: auto;
  padding-top: 12px;
  border-top: 1px solid var(--border-color);
}

.project-foot :deep(.el-button + .el-button) {
  margin-left: 0;
}

.foot-spacer {
  flex: 1;
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

.full-width {
  width: 100%;
}

.card-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-top: 16px;
}

.field-hint {
  margin: 4px 0 0;
  font-size: 11px;
  line-height: 1.6;
  color: var(--text-muted);
}

.empty-state {
  min-height: 110px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: center;
  justify-content: center;
  padding: 18px;
  color: var(--text-muted);
  font-size: 12px;
  text-align: center;
  border: 1px dashed var(--border-color);
  border-radius: var(--radius-sm);
}

.empty-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
}

.empty-hint {
  max-width: 520px;
  line-height: 1.6;
}
</style>
