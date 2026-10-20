<template>
  <div class="card-surface registry-sources-card">
    <div class="card-title-bar">
      <div>
        <h3>镜像源管理</h3>
        <p class="field-hint">管理公开或私有镜像仓库，拉取镜像时可选择使用。</p>
      </div>
      <div class="card-actions">
        <el-button :loading="loading" :disabled="disabled" @click="load"><el-icon><Refresh /></el-icon><span>刷新</span></el-button>
        <el-button type="primary" :disabled="disabled" @click="edit(null)"><el-icon><Plus /></el-icon><span>新增镜像源</span></el-button>
      </div>
    </div>
    <el-alert v-if="errorMessage" :title="errorMessage" type="error" :closable="false" show-icon class="source-alert" />
    <el-table v-if="sources.length" :data="sources" row-key="id" size="small">
      <el-table-column prop="name" label="名称" min-width="150" />
      <el-table-column label="仓库地址" min-width="230">
        <template #default="{ row }"><code>{{ row.address }}</code></template>
      </el-table-column>
      <el-table-column label="命名空间" min-width="140">
        <template #default="{ row }">{{ row.namespace || '未指定' }}</template>
      </el-table-column>
      <el-table-column label="访问方式" min-width="150">
        <template #default="{ row }">
          <span>{{ row.username || '无需登录' }}</span>
          <div v-if="row.username" class="field-hint">{{ row.hasPassword ? '已保存密码' : '每次输入密码' }}</div>
        </template>
      </el-table-column>
      <el-table-column label="操作" width="230" fixed="right">
        <template #default="{ row }">
          <el-button link type="primary" :disabled="disabled || busy(row.id)" @click="edit(row)">编辑</el-button>
          <el-button link type="primary" :loading="testingId === row.id" :disabled="disabled || Boolean(testingId || deletingId)" @click="test(row)">测试本机连接</el-button>
          <el-button link type="danger" :disabled="disabled || busy(row.id)" @click="remove(row)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>
    <div v-else-if="!loading && !errorMessage" class="empty-state">
      <span class="empty-title">还没有配置镜像源</span>
      <span>添加仓库地址，需要登录的私有仓库还可保存用户名和密码。</span>
    </div>
    <div v-else-if="loading" class="empty-state">正在读取镜像源</div>
    <el-alert v-if="testResult" :title="testResult.message" :type="testResult.success ? 'success' : 'error'" :closable="false" show-icon class="source-alert result-alert" />
    <RegistrySourceEditor v-model="editorVisible" :source="editing" @saved="saved" />
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Plus, Refresh } from '@element-plus/icons-vue';
import RegistrySourceEditor from './RegistrySourceEditor.vue';
import { deleteRegistrySource, listRegistrySources, testRegistrySource } from '../../../api/tauri.js';

const emit = defineEmits(['changed']);
const props = defineProps({ disabled: Boolean });
const sources = ref([]);
const loading = ref(false);
const errorMessage = ref('');
const editorVisible = ref(false);
const editing = ref(null);
const testingId = ref('');
const deletingId = ref('');
const testResult = ref(null);
const busy = (id) => testingId.value === id || deletingId.value === id;

async function load() {
  loading.value = true;
  errorMessage.value = '';
  try {
    sources.value = await listRegistrySources();
    emit('changed', sources.value);
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error);
  } finally {
    loading.value = false;
  }
}

function edit(source) {
  if (props.disabled) return;
  editing.value = source;
  editorVisible.value = true;
}

async function saved() {
  testResult.value = null;
  await load();
}

async function test(source) {
  if (props.disabled || testingId.value || deletingId.value) return;
  testingId.value = source.id;
  testResult.value = null;
  try {
    const result = await testRegistrySource(source.id);
    testResult.value = { success: true, message: `${source.name}：${result.message}` };
  } catch (error) {
    testResult.value = { success: false, message: `${source.name}：${error instanceof Error ? error.message : String(error)}` };
  } finally {
    testingId.value = '';
  }
}

async function remove(source) {
  if (props.disabled || busy(source.id)) return;
  try {
    await ElMessageBox.confirm(`确认删除镜像源「${source.name}」（${source.address}）？已保存的密码会同时清除，本机镜像与离线包不受影响。`, '删除镜像源', { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' });
  } catch {
    return;
  }
  deletingId.value = source.id;
  try {
    await deleteRegistrySource(source.id);
    testResult.value = null;
    ElMessage.success('镜像源已删除');
    await load();
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : String(error));
  } finally {
    deletingId.value = '';
  }
}

onMounted(load);
</script>

<style scoped>
.registry-sources-card { padding: 20px; margin-bottom: 18px; }
.card-title-bar { display: flex; justify-content: space-between; align-items: center; gap: 14px; margin-bottom: 18px; flex-wrap: wrap; }
.card-title-bar h3 { font-size: 15px; color: var(--text-main); }
.card-actions { display: flex; gap: 10px; flex-wrap: wrap; }
.card-actions .el-button + .el-button { margin-left: 0; }
.field-hint { margin: 5px 0 0; font-size: 12px; color: var(--text-muted); line-height: 1.6; }
.source-alert { margin-bottom: 14px; }
.result-alert { margin-top: 16px; margin-bottom: 0; }
.empty-state { min-height: 120px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; color: var(--text-muted); font-size: 13px; padding: 20px; text-align: center; border: 1px dashed var(--border-color); border-radius: var(--radius-sm); }
.empty-title { font-weight: 600; color: var(--text-main); }
code { overflow-wrap: anywhere; }
</style>
