<template>
  <div class="page-view credentials-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">凭证管理</h2>
        <p class="page-subtitle">维护可复用的 SSH 登录身份，供主机集中绑定</p>
      </div>
      <div class="header-actions">
        <el-button @click="$emit('go-hosts')">前往主机管理</el-button>
      </div>
    </div>

    <div class="card-surface credentials-card">
      <div class="card-title-bar">
        <div>
          <h3>SSH 凭证</h3>
          <p class="card-subtitle">私钥仍保留在本机原位置，密码保存后不再显示原文</p>
        </div>
        <el-tag size="small" effect="plain">{{ credentials.length }} 个凭证</el-tag>
      </div>

      <el-alert
        v-if="connectionInventory.error"
        :title="connectionInventory.error"
        type="error"
        :closable="false"
        show-icon
        class="page-alert"
      />

      <div v-if="credentials.length" class="credential-list">
        <article v-for="row in credentials" :key="row.id" class="credential-card">
          <div class="credential-head">
            <span class="credential-tile"><el-icon><Key /></el-icon></span>
            <div class="credential-ident">
              <span class="credential-name">{{ row.name }}</span>
              <span class="credential-user">登录用户 {{ row.username }}</span>
            </div>
            <el-tag :type="row.hasPassword ? 'success' : 'info'" size="small" effect="plain">
              {{ row.hasPassword ? '已存初始密码' : '无初始密码' }}
            </el-tag>
          </div>

          <div class="credential-body">
            <div class="info-row">
              <span class="k">私钥路径</span>
              <span
                class="v"
                :class="row.privateKeyPath ? 'code' : 'muted'"
                :title="row.privateKeyPath || '自动选择默认密钥'"
              >{{ row.privateKeyPath || '自动选择默认密钥' }}</span>
            </div>
            <div class="info-row">
              <span class="k">关联主机</span>
              <span class="v" :class="hostCount(row.id) ? '' : 'muted'">
                {{ hostCount(row.id) ? `${hostCount(row.id)} 台` : '未关联' }}
              </span>
            </div>
            <div class="info-row">
              <span class="k">更新于</span>
              <span class="v">{{ formatTime(row.updatedAt) }}</span>
            </div>
          </div>

          <div class="credential-foot">
            <span class="foot-hint">
              {{ hostCount(row.id) ? '更换关联主机的凭证后可删除' : '' }}
            </span>
            <div class="foot-actions">
              <el-button size="small" text @click="openEdit(row)">编辑</el-button>
              <el-button
                size="small"
                text
                type="danger"
                :disabled="hostCount(row.id) > 0"
                @click="remove(row)"
              >删除</el-button>
            </div>
          </div>
        </article>
      </div>

      <div v-else class="empty-state">
        <span class="empty-title">还没有 SSH 凭证</span>
        <span class="empty-hint">先保存登录用户名与私钥位置，再到主机管理中绑定目标服务器。</span>
      </div>

      <div class="card-actions">
        <el-button type="primary" @click="openCreate">
          <el-icon><Plus /></el-icon>
          <span>新建凭证</span>
        </el-button>
        <el-button :loading="connectionInventory.loading" @click="load">
          <el-icon><Refresh /></el-icon>
          <span>刷新</span>
        </el-button>
      </div>
    </div>

    <CredentialEditor
      v-model="editorVisible"
      :credential="editing"
      @saved="load"
    />
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Key, Plus, Refresh } from '@element-plus/icons-vue';
import CredentialEditor from '../components/CredentialEditor.vue';
import { connectionInventory, refreshConnectionInventory } from '../state.js';
import { deleteSshCredential } from '../../../api/tauri.js';

defineEmits(['go-hosts']);

const editorVisible = ref(false);
const editing = ref(null);
const credentials = computed(() => connectionInventory.credentials);

function hostCount(credentialId) {
  return connectionInventory.hosts.filter((host) => host.credentialId === credentialId).length;
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

async function load() {
  try {
    await refreshConnectionInventory();
  } catch (error) {
    ElMessage.error(String(error));
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

async function remove(row) {
  try {
    await ElMessageBox.confirm(
      `确认删除凭证「${row.name}」？\n\n已保存的初始密码会一并清除，此操作无法撤销。`,
      '删除 SSH 凭证',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' }
    );
  } catch {
    return;
  }
  try {
    await deleteSshCredential(row.id);
    ElMessage.success('凭证已删除');
    await load();
  } catch (error) {
    ElMessage.error(String(error));
  }
}

onMounted(load);
</script>

<style scoped>
.credentials-card {
  padding: 20px;
}

.card-subtitle {
  margin-top: 5px;
  font-size: 11px;
  color: var(--text-muted);
}

.page-alert {
  margin-bottom: 14px;
}

.credential-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 14px;
}

.credential-card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
  transition: border-color 0.18s ease, box-shadow 0.18s ease;
}

.credential-card:hover {
  border-color: #c9d2e3;
  box-shadow: var(--shadow-raised);
}

.credential-head {
  display: flex;
  align-items: flex-start;
  gap: 11px;
  padding: 15px 16px 13px;
  border-bottom: 1px solid var(--border-color);
}

.credential-tile {
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  border-radius: 7px;
  color: var(--primary-color);
  background: var(--primary-soft);
}

.credential-ident {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  gap: 3px;
}

.credential-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
  overflow-wrap: anywhere;
}

.credential-user {
  font-size: 11px;
  color: var(--text-muted);
}

.credential-body {
  display: flex;
  flex-direction: column;
  gap: 9px;
  padding: 13px 16px;
  font-size: 12px;
}

.info-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 14px;
}

.info-row .k {
  flex: none;
  color: var(--text-muted);
}

.info-row .v {
  min-width: 0;
  text-align: right;
  color: var(--text-main);
  overflow-wrap: anywhere;
}

.info-row .v.code {
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  font-size: 11px;
  color: var(--text-secondary);
}

.info-row .v.muted {
  color: var(--text-muted);
}

.credential-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-height: 41px;
  margin-top: auto;
  padding: 6px 12px;
  border-top: 1px solid var(--border-color);
  background: var(--panel-subtle);
}

.foot-hint {
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.4;
}

.foot-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: none;
  margin-left: auto;
}

.foot-actions :deep(.el-button + .el-button) {
  margin-left: 0;
}

.card-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 16px;
}

.empty-state {
  min-height: 170px;
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
