<template>
  <div class="step-card">
    <h3>确认部署目标</h3>
    <p class="desc">选择要部署的项目，项目决定目标主机与部署目录。</p>
    <el-alert
      v-if="inventoryError"
      title="无法读取主机列表，请在主机管理中检查后重试。"
      type="error"
      :closable="false"
      show-icon
    />
    <div v-if="!hasProject" class="project-empty">
      <span class="icon-tile neutral"><el-icon><FolderOpened /></el-icon></span>
      <span class="empty-title">当前没有可用项目</span>
      <span class="empty-hint">
        一个项目对应一台主机、一个部署目录和一个制品源。新建项目后即可选择制品并部署。
      </span>
      <el-button type="primary" @click="emit('go-projects')">前往项目管理</el-button>
    </div>
    <el-form v-else label-width="110px" class="host-form" @submit.prevent>
      <el-form-item label="项目">
        <el-select
          :model-value="selectedProjectId"
          class="project-select"
          placeholder="请选择项目"
          :disabled="busy || switchingProject"
          @change="(id) => emit('select-project', id)"
        >
          <el-option v-for="item in projects" :key="item.id" :value="item.id" :label="item.name" />
        </el-select>
      </el-form-item>
      <el-form-item label="目标主机">
        <div class="input-row">
          <span>{{ targetLabel }}</span>
          <el-button :disabled="busy" @click="emit('go-projects')">项目管理</el-button>
        </div>
      </el-form-item>
      <el-form-item label="部署目录">
        <span class="path-text">{{ installDir || '未设置' }}</span>
      </el-form-item>
      <el-form-item v-if="deployDir && deployDir !== installDir" label="Compose 目录">
        <span class="path-text">{{ deployDir }}</span>
      </el-form-item>
      <el-form-item v-if="isRemote" label="连接检查">
        <div class="input-row wrap">
          <el-button :loading="testingSsh" :disabled="busy" @click="emit('test-connection')">
            测试连接
          </el-button>
          <el-button :loading="settingKey" :disabled="busy" @click="emit('setup-access')">
            配置免密登录
          </el-button>
          <span v-if="sshMessage" :class="sshOk ? 'text-success' : 'text-danger'">{{ sshMessage }}</span>
        </div>
      </el-form-item>
    </el-form>
    <div class="step-footer">
      <span class="field-hint">{{ hasProject ? `部署目标：${targetLabel}` : '请先创建项目' }}</span>
      <el-button type="primary" :disabled="!hostReady || busy" @click="emit('next')">
        下一步：检测环境
        <el-icon class="el-icon--right"><ArrowRight /></el-icon>
      </el-button>
    </div>
  </div>
</template>

<script setup>
import { ArrowRight, FolderOpened } from '@element-plus/icons-vue';

defineProps({
  hasProject: { type: Boolean, default: false },
  projects: { type: Array, default: () => [] },
  selectedProjectId: { type: String, default: '' },
  switchingProject: { type: Boolean, default: false },
  busy: { type: Boolean, default: false },
  targetLabel: { type: String, default: '' },
  installDir: { type: String, default: '' },
  deployDir: { type: String, default: '' },
  isRemote: { type: Boolean, default: false },
  inventoryError: { type: String, default: '' },
  testingSsh: { type: Boolean, default: false },
  settingKey: { type: Boolean, default: false },
  sshMessage: { type: String, default: '' },
  sshOk: { type: Boolean, default: false },
  hostReady: { type: Boolean, default: false }
});

const emit = defineEmits([
  'go-projects',
  'select-project',
  'test-connection',
  'setup-access',
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
.project-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 32px 20px;
  border: 1px dashed var(--border-color);
  border-radius: var(--radius-md);
  text-align: center;
  color: var(--text-muted);
  background: var(--panel-subtle);
}
.project-empty .empty-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-main);
}
.project-empty .empty-hint {
  max-width: 520px;
  font-size: 12px;
  line-height: 1.6;
}
.path-text {
  font-family: ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace;
  font-size: 12px;
  color: var(--text-main);
  background: var(--surface-muted);
  padding: 2px 8px;
  border-radius: var(--radius-xs);
  border: 1px solid var(--border-subtle);
  overflow-wrap: anywhere;
}
.host-form {
  margin-top: 24px;
  max-width: 850px;
}
.project-select {
  width: 100%;
  max-width: 360px;
}
.input-row {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
}
.wrap {
  flex-wrap: wrap;
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
@media (max-width: 760px) {
  .step-card {
    padding: 20px;
  }
  .step-footer {
    flex-wrap: wrap;
  }
}
</style>
