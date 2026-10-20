<template>
  <div class="page-view hosts-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">主机管理</h2>
        <p class="page-subtitle">集中保存本机与远程服务器目标，部署、更新和诊断共用当前主机</p>
      </div>
      <div class="header-actions">
        <el-button @click="$emit('go-credentials')">管理凭证</el-button>
      </div>
    </div>

    <div class="card-surface hosts-card">
      <div class="card-title-bar">
        <div>
          <h3>目标主机</h3>
          <p class="card-subtitle">当前主机决定部署、更新、监控与日志诊断在本机还是远程服务器执行</p>
        </div>
        <el-tag size="small" effect="plain">{{ hosts.length }} 台主机</el-tag>
      </div>

      <el-alert
        v-if="connectionInventory.error"
        :title="connectionInventory.error"
        type="error"
        :closable="false"
        show-icon
        class="page-alert"
      />

      <div v-if="hosts.length" class="host-list">
        <article
          v-for="row in hosts"
          :key="row.id"
          class="host-card"
          :class="{ 'is-active': row.id === connectionInventory.activeHostId }"
        >
          <div class="host-head">
            <span class="host-tile">
              <el-icon><Monitor v-if="isLocalHost(row)" /><Connection v-else /></el-icon>
            </span>
            <div class="host-ident">
              <div class="host-name-line">
                <span class="host-name">{{ row.name }}</span>
                <el-tag v-if="isLocalHost(row)" size="small" type="info" effect="plain">本机</el-tag>
                <el-tag v-if="row.id === connectionInventory.activeHostId" size="small" type="success" effect="plain">
                  当前主机
                </el-tag>
              </div>
              <span v-if="row.notes" class="host-notes">{{ row.notes }}</span>
            </div>
            <el-button
              v-if="row.id !== connectionInventory.activeHostId"
              size="small"
              type="primary"
              plain
              :loading="busyKey === `activate:${row.id}`"
              @click="activate(row)"
            >设为当前</el-button>
          </div>

          <div class="host-body">
            <template v-if="isLocalHost(row)">
              <div class="info-row">
                <span class="k">目标位置</span>
                <span class="v">当前电脑</span>
              </div>
              <div class="info-row">
                <span class="k">连接方式</span>
                <span class="v">本机直接执行</span>
              </div>
            </template>
            <template v-else>
              <div class="info-row">
                <span class="k">SSH 地址</span>
                <span class="v code">{{ row.address }}:{{ row.port }}</span>
              </div>
              <div class="info-row">
                <span class="k">登录凭证</span>
                <div class="v credential-value">
                  <span>{{ credentialLabel(row.credentialId) }}</span>
                  <span class="credential-key">{{ credentialKeyText(row.credentialId) }}</span>
                </div>
              </div>
            </template>
            <div class="info-row">
              <span class="k">部署目录</span>
              <span class="v" :class="row.deployDir ? 'code' : 'muted'">
                {{ row.deployDir || '沿用当前配置' }}
              </span>
            </div>
            <div class="info-row">
              <span class="k">更新于</span>
              <span class="v muted">{{ formatTime(row.updatedAt) }}</span>
            </div>
          </div>

          <div class="host-foot">
            <div class="foot-actions">
              <el-button size="small" text type="primary" @click="$emit('view-status', row)">状态</el-button>
              <el-button
                v-if="!isLocalHost(row)"
                size="small"
                text
                :loading="busyKey === `test:${row.id}`"
                @click="testHost(row)"
              >测试连接</el-button>
              <el-button
                v-if="!isLocalHost(row)"
                size="small"
                text
                :loading="busyKey === `setup:${row.id}`"
                @click="setupAccess(row)"
              >配置免密</el-button>
              <el-button size="small" text @click="openEdit(row)">编辑</el-button>
              <el-button size="small" text type="danger" @click="remove(row)">删除</el-button>
            </div>
          </div>
        </article>
      </div>

      <div v-else class="empty-state">
        <span class="empty-title">还没有保存的主机</span>
        <span class="empty-hint">可添加本机（当前电脑）或通过 SSH 连接局域网服务器，保存后用于部署、更新、监控和日志诊断。</span>
      </div>

      <div class="card-actions">
        <el-button type="primary" @click="openCreate">
          <el-icon><Plus /></el-icon>
          <span>新增主机</span>
        </el-button>
        <el-button :loading="connectionInventory.loading" @click="load">
          <el-icon><Refresh /></el-icon>
          <span>刷新</span>
        </el-button>
      </div>
    </div>

    <HostEditor
      v-model="editorVisible"
      :host="editing"
      :credentials="credentials"
      :hosts="hosts"
      :default-deploy-dir="connection.deployDir"
      @saved="load"
    />

    <el-dialog v-model="resultVisible" :title="resultTitle" width="560px">
      <div class="result-body" :class="resultOk ? 'success' : 'failure'">
        <el-icon><CircleCheck v-if="resultOk" /><CircleClose v-else /></el-icon>
        <pre>{{ resultMessage }}</pre>
      </div>
      <template #footer>
        <el-button type="primary" @click="resultVisible = false">知道了</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { CircleCheck, CircleClose, Connection, Monitor, Plus, Refresh } from '@element-plus/icons-vue';
import HostEditor from '../components/HostEditor.vue';
import {
  applyConnectionInventory,
  connection,
  connectionInventory,
  credentialById,
  credentialLabel,
  isLocalHost,
  refreshConnectionInventory,
  useManagedHost
} from '../connection.js';
import {
  deleteManagedHost,
  setupManagedHostAccess,
  testManagedHostConnection
} from '../api/tauri.js';

defineEmits(['go-credentials', 'view-status']);

const editorVisible = ref(false);
const editing = ref(null);
const busyKey = ref('');
const resultVisible = ref(false);
const resultTitle = ref('');
const resultMessage = ref('');
const resultOk = ref(false);

const hosts = computed(() => connectionInventory.hosts);
const credentials = computed(() => connectionInventory.credentials);

function credentialKeyText(credentialId) {
  const credential = credentialById(credentialId);
  if (!credential) return '凭证不可用';
  return credential.privateKeyPath || '自动选择默认密钥';
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

async function activate(row) {
  busyKey.value = `activate:${row.id}`;
  try {
    await useManagedHost(row.id);
    ElMessage.success(`当前主机已切换为「${row.name}」`);
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    busyKey.value = '';
  }
}

function showResult(title, message, success) {
  resultTitle.value = title;
  resultMessage.value = message;
  resultOk.value = success;
  resultVisible.value = true;
}

async function testHost(row) {
  busyKey.value = `test:${row.id}`;
  try {
    const result = await testManagedHostConnection(row.id);
    showResult(
      result.success ? '连接正常' : '连接失败',
      result.message,
      result.success
    );
  } catch (error) {
    showResult('连接失败', String(error), false);
  } finally {
    busyKey.value = '';
  }
}

async function setupAccess(row) {
  busyKey.value = `setup:${row.id}`;
  try {
    const result = await setupManagedHostAccess(row.id);
    showResult(
      result.success ? '免密登录已就绪' : '免密配置未完成',
      result.message,
      result.success
    );
  } catch (error) {
    showResult('免密配置未完成', String(error), false);
  } finally {
    busyKey.value = '';
  }
}

async function remove(row) {
  try {
    await ElMessageBox.confirm(
      `确认删除主机「${row.name}」？\n\n不会影响已部署的服务；删除当前主机后会自动切换到其他已保存主机，没有其他主机时回到本机部署。`,
      '删除主机',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' }
    );
  } catch {
    return;
  }
  try {
    const inventory = await deleteManagedHost(row.id);
    applyConnectionInventory(inventory);
    ElMessage.success('主机已删除');
  } catch (error) {
    ElMessage.error(String(error));
  }
}

onMounted(load);
</script>

<style scoped>
.hosts-card {
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

.host-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 14px;
}

.host-card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
  transition: border-color 0.18s ease, box-shadow 0.18s ease;
}

.host-card:hover {
  border-color: #c9d2e3;
  box-shadow: var(--shadow-raised);
}

.host-card.is-active {
  border-color: rgba(22, 119, 255, 0.45);
}

.host-head {
  display: flex;
  align-items: flex-start;
  gap: 11px;
  padding: 15px 16px 13px;
  border-bottom: 1px solid var(--border-color);
}

.host-tile {
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

.host-ident {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  gap: 3px;
}

.host-name-line {
  display: flex;
  align-items: center;
  gap: 7px;
  flex-wrap: wrap;
}

.host-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
  overflow-wrap: anywhere;
}

.host-notes {
  color: var(--text-muted);
  font-size: 11px;
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.host-body {
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

.credential-value {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.credential-key {
  color: var(--text-muted);
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  font-size: 10px;
  overflow-wrap: anywhere;
}

.host-foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  min-height: 41px;
  margin-top: auto;
  padding: 6px 12px;
  border-top: 1px solid var(--border-color);
  background: var(--panel-subtle);
}

.foot-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 2px;
  flex-wrap: wrap;
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
  min-height: 190px;
  display: flex;
  flex-direction: column;
  gap: 9px;
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
  margin-bottom: 3px;
}

.result-body {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 12px 14px;
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
}

.result-body .el-icon {
  flex: none;
  margin-top: 2px;
  font-size: 18px;
}

.result-body.success .el-icon {
  color: var(--success-color);
}

.result-body.failure .el-icon {
  color: var(--danger-color);
}

.result-body pre {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-family: inherit;
  font-size: 12px;
  line-height: 1.65;
  color: var(--text-secondary);
}
</style>
