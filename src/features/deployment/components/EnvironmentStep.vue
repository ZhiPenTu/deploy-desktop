<template>
  <div class="step-card" v-loading="checkingEnv">
    <div class="section-heading">
      <div>
        <h3>检测目标环境</h3>
        <p class="desc">检查 {{ targetLabel }} 的系统资源、Docker、Compose 与端口占用。</p>
      </div>
      <el-button :loading="checkingEnv" :disabled="busy" @click="emit('recheck')"> 重新检测 </el-button>
    </div>
    <el-alert v-if="envError" :title="envError" type="error" :closable="false" show-icon />
    <template v-if="envData">
      <div class="env-metrics">
        <div class="metric-card">
          <span>操作系统 / 架构</span>
          <strong>{{ envData.os_release }} / {{ envData.arch }}</strong>
        </div>
        <div class="metric-card">
          <span>CPU 核心</span>
          <strong>{{ envData.cpu_cores }} 核</strong>
        </div>
        <div class="metric-card">
          <span>物理内存</span>
          <strong>{{ envData.total_memory_mb }} MB</strong>
        </div>
        <div class="metric-card">
          <span>可用磁盘</span>
          <strong>{{ envData.disk_available_gb }} GB</strong>
        </div>
      </div>
      <div class="status-row">
        <el-tag :type="envData.docker_running ? 'success' : 'danger'">
          {{ envData.docker_running ? 'Docker 正常运行' : 'Docker 未就绪' }}
        </el-tag>
        <el-tag :type="envData.compose_installed ? 'success' : 'danger'">
          {{ envData.compose_installed ? 'Compose 已安装' : 'Compose 未安装' }}
        </el-tag>
      </div>
      <ul v-if="envData.issues?.length" class="issue-list">
        <li v-for="issue in envData.issues" :key="issue">
          {{ safeText(issue) }}
        </li>
      </ul>
      <el-collapse v-if="!envData.docker_running" class="optional-panel">
        <el-collapse-item title="安装离线 Docker 引擎" name="docker">
          <p class="field-hint">
            请填写{{ isRemote ? '目标主机' : '本机' }}上的离线介质目录，目录中需包含 Docker 安装包。
          </p>
          <div class="input-row">
            <el-input
              :model-value="mediaDir"
              placeholder="离线介质目录"
              :disabled="busy"
              @update:model-value="(value) => emit('update:media-dir', value)"
            />
            <el-button
              :loading="installingDocker"
              :disabled="!mediaDir || busy"
              @click="emit('install-docker')"
            >
              安装并重新检测
            </el-button>
          </div>
        </el-collapse-item>
      </el-collapse>
      <el-table :data="envData.ports || []" size="small" empty-text="暂无需检查的端口">
        <el-table-column prop="port" label="端口" width="90" />
        <el-table-column prop="protocol" label="协议" width="80" />
        <el-table-column prop="service" label="服务" min-width="170" />
        <el-table-column label="状态" width="110">
          <template #default="{ row }">
            <el-tag :type="row.available ? 'success' : 'warning'" size="small">
              {{ row.available ? '空闲' : '占用中' }}
            </el-tag>
          </template>
        </el-table-column>
      </el-table>
    </template>
    <div class="step-footer">
      <el-button :disabled="busy" @click="emit('previous')"> 上一步 </el-button>
      <el-button type="primary" :disabled="!environmentReady || busy" @click="emit('next')">
        下一步：选择制品
        <el-icon class="el-icon--right"><ArrowRight /></el-icon>
      </el-button>
    </div>
  </div>
</template>

<script setup>
import { ArrowRight } from '@element-plus/icons-vue';
import { safeText } from '../domain/errors.js';

defineProps({
  targetLabel: { type: String, default: '' },
  checkingEnv: { type: Boolean, default: false },
  busy: { type: Boolean, default: false },
  envError: { type: String, default: '' },
  envData: { type: Object, default: null },
  isRemote: { type: Boolean, default: false },
  mediaDir: { type: String, default: '' },
  installingDocker: { type: Boolean, default: false },
  environmentReady: { type: Boolean, default: false }
});

const emit = defineEmits([
  'recheck',
  'install-docker',
  'update:media-dir',
  'previous',
  'next'
]);
</script>

<style scoped>
.step-card {
  padding: 26px 28px;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
}
h3 {
  font-size: 16px;
  font-weight: 650;
  letter-spacing: -0.01em;
  margin-bottom: 6px;
  color: var(--text-main);
}
.desc {
  color: var(--text-muted);
  font-size: 12.5px;
  line-height: 1.6;
  margin-bottom: 22px;
}
.section-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}
.status-row {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  margin: 16px 0;
  color: var(--text-secondary);
  font-size: 12.5px;
}
.input-row {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
}
.input-row .el-input {
  flex: 1;
  min-width: 150px;
}
.step-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-top: 22px;
  margin-top: 26px;
  border-top: 1px solid var(--border-color);
}
.field-hint {
  width: 100%;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.65;
  margin-top: 6px;
  overflow-wrap: anywhere;
}
.env-metrics {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 14px;
  margin: 20px 0 24px;
}
.metric-card {
  padding: 16px 18px;
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  display: flex;
  flex-direction: column;
  gap: 8px;
  transition: border-color 0.18s ease, box-shadow 0.18s ease;
}
.metric-card:hover {
  border-color: var(--brand-border);
  box-shadow: var(--shadow-card);
}
.metric-card span {
  color: var(--text-muted);
  font-size: 11.5px;
  font-weight: 500;
}
.metric-card strong {
  font-size: 15px;
  font-weight: 650;
  color: var(--text-main);
  letter-spacing: -0.01em;
  word-break: break-word;
}
.issue-list {
  padding: 14px 18px 14px 34px;
  margin: 16px 0;
  color: var(--warning-color);
  background: var(--warning-soft);
  border: 1px solid rgba(245, 158, 11, 0.2);
  border-radius: var(--radius-sm);
  font-size: 12px;
  line-height: 1.8;
}
.optional-panel {
  margin-top: 20px;
}
.optional-panel .input-row {
  margin: 12px 0;
}
@media (max-width: 1100px) {
  .env-metrics {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@media (max-width: 760px) {
  .step-card {
    padding: 20px;
  }
  .step-footer,
  .section-heading {
    flex-wrap: wrap;
  }
}
</style>
