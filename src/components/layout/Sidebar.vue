<template>
  <aside class="app-sidebar">
    <div class="sidebar-drag" data-tauri-drag-region></div>
    <div class="brand-block" data-tauri-drag-region="deep">
      <img class="brand-mark" :src="appIcon" alt="" />
      <div class="brand-copy">
        <h1>部署舱</h1>
        <p>Deployment Console</p>
      </div>
    </div>

    <div class="project-block">
      <span class="project-label">当前项目</span>
      <template v-if="projects.length">
        <select
          class="project-select"
          :value="activeProjectId || undefined"
          :disabled="switching"
          aria-label="切换当前项目"
          @change="$emit('switch-project', $event.target.value)"
        >
          <option v-for="item in projects" :key="item.id" :value="item.id">{{ item.name }}</option>
        </select>
        <span class="project-meta">{{ projectMeta }}</span>
      </template>
      <button v-else type="button" class="project-empty" @click="$emit('update:tab', 'projects')">
        尚无项目，前往创建
      </button>
    </div>

    <div class="nav-scroll">
      <div v-for="group in navGroups" :key="group.label" class="nav-group">
        <div class="nav-label">{{ group.label }}</div>
        <nav class="nav-menu">
          <button
            v-for="item in group.items"
            :key="item.key"
            type="button"
            class="nav-item"
            :class="{ active: currentTab === item.key }"
            @click="$emit('update:tab', item.key)"
          >
            <el-icon :size="16"><component :is="item.icon" /></el-icon>
            <span class="title">{{ item.title }}</span>
            <span v-if="item.badge" class="badge">{{ item.badge }}</span>
          </button>
        </nav>
      </div>
    </div>

    <div class="sidebar-footer">
      <div class="run-state">
        <span class="state-dot"></span>
        <span>本机离线运行</span>
      </div>
      <span class="version">v{{ appVersion }}</span>
    </div>
  </aside>
</template>

<script setup>
import {
  Promotion,
  DataBoard,
  Document,
  Files,
  Box,
  Connection,
  Setting,
  Key,
  Monitor,
  FolderOpened
} from '@element-plus/icons-vue';
import appIcon from '../../assets/app-icon.png';
import { computed } from 'vue';
import { projectState } from '../../config/project.js';
import { hostById, isLocalHost } from '../../features/connections/state.js';

const appVersion = __APP_VERSION__;

defineProps({
  currentTab: {
    type: String,
    default: 'wizard'
  },
  switching: {
    type: Boolean,
    default: false
  }
});

defineEmits(['update:tab', 'switch-project']);

const projects = computed(() => projectState.projects);
const activeProjectId = computed(() => projectState.project?.id || '');
const projectMeta = computed(() => {
  const project = projectState.project;
  if (!project) return '';
  const host = hostById(project.hostId);
  const hostText = host ? (isLocalHost(host) ? '本机' : host.address || host.name) : '主机未配置';
  const version = projectState.bundle?.version ? ` · ${projectState.bundle.version}` : '';
  return `${hostText}${version}`;
});

const navGroups = [
  {
    label: '项目',
    items: [
      { key: 'projects', icon: FolderOpened, title: '项目管理' }
    ]
  },
  {
    label: '连接',
    items: [
      { key: 'credentials', icon: Key, title: '凭证管理' },
      { key: 'hosts', icon: Monitor, title: '主机管理' }
    ]
  },
  {
    label: '部署',
    items: [
      { key: 'wizard', icon: Promotion, title: '部署向导' }
    ]
  },
  {
    label: '运行',
    items: [
      { key: 'dashboard', icon: DataBoard, title: '集群监控' },
      { key: 'logs', icon: Document, title: '日志诊断' }
    ]
  },
  {
    label: '资源',
    items: [
      { key: 'artifacts', icon: Box, title: '制品中心' },
      { key: 'images', icon: Files, title: '镜像与离线包' }
    ]
  },
  {
    label: '高级',
    items: [
      { key: 'mcp', icon: Connection, title: 'MCP 服务' },
      { key: 'settings', icon: Setting, title: '设置' }
    ]
  }
];
</script>

<style scoped>
.app-sidebar {
  width: 232px;
  flex: none;
  display: flex;
  flex-direction: column;
  background: var(--sidebar-bg);
  color: var(--sidebar-text);
  border-right: 1px solid rgba(15, 23, 42, 0.9);
}

.sidebar-drag {
  display: none;
  height: 34px;
  flex: none;
}

.platform-macos .sidebar-drag {
  display: block;
}

.brand-block {
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 20px 18px 18px;
  border-bottom: 1px solid var(--sidebar-border);
}

.platform-macos .brand-block {
  padding-top: 4px;
}

.brand-mark {
  width: 34px;
  height: 34px;
  flex: none;
  display: block;
  border-radius: 9px;
  box-shadow: 0 6px 16px rgba(51, 92, 255, 0.35);
}

.brand-copy {
  min-width: 0;
}

.brand-copy h1 {
  font-size: 13.5px;
  font-weight: 600;
  letter-spacing: 0.01em;
  color: #f8fafc;
  white-space: nowrap;
}

.brand-copy p {
  margin-top: 2px;
  font-size: 10px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--sidebar-muted);
  white-space: nowrap;
}

.project-block {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--sidebar-border);
}

.project-label {
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.12em;
  color: #64748b;
}

.project-select {
  width: 100%;
  height: 32px;
  padding: 0 8px;
  border: 1px solid rgba(120, 150, 255, 0.22);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.06);
  color: #f1f5f9;
  font: inherit;
  font-size: 12.5px;
  cursor: pointer;
  appearance: none;
}

.project-select option {
  color: #0f172a;
  background: #ffffff;
}

.project-select:disabled {
  opacity: 0.6;
  cursor: default;
}

.project-meta {
  font-size: 11px;
  color: #7f8ea6;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.project-empty {
  width: 100%;
  min-height: 32px;
  padding: 6px 10px;
  border: 1px dashed rgba(120, 150, 255, 0.35);
  border-radius: 8px;
  background: rgba(51, 92, 255, 0.08);
  color: #c3d0e2;
  font: inherit;
  font-size: 12px;
  text-align: left;
  cursor: pointer;
}

.project-empty:hover {
  background: rgba(51, 92, 255, 0.16);
  color: #f1f5f9;
}

.nav-scroll {
  flex: 1;
  padding: 12px 12px 16px;
  overflow-y: auto;
}

.nav-group + .nav-group {
  margin-top: 18px;
}

.nav-label {
  padding: 0 10px;
  margin-bottom: 7px;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.12em;
  color: #64748b;
}

.nav-menu {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.nav-item {
  position: relative;
  width: 100%;
  height: 36px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 11px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: #a8b5c7;
  font: inherit;
  font-size: 13px;
  text-align: left;
  cursor: pointer;
  transition: background-color 0.16s ease, color 0.16s ease, border-color 0.16s ease;
}

.nav-item .el-icon {
  flex: none;
  color: #7688a0;
  transition: color 0.16s ease;
}

.nav-item:hover {
  background: rgba(255, 255, 255, 0.05);
  color: #f1f5f9;
}

.nav-item:hover .el-icon {
  color: #c3d0e2;
}

.nav-item.active {
  color: #ffffff;
  font-weight: 500;
  background: linear-gradient(90deg, rgba(51, 92, 255, 0.26) 0%, rgba(51, 92, 255, 0.08) 100%);
  border-color: rgba(120, 150, 255, 0.22);
}

.nav-item.active::before {
  content: "";
  position: absolute;
  left: -1px;
  top: 9px;
  bottom: 9px;
  width: 3px;
  border-radius: 3px;
  background: #6f8dff;
}

.nav-item.active .el-icon {
  color: var(--sidebar-active);
}

.nav-item .title {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.badge {
  font-size: 10px;
  font-weight: 600;
  color: #d6b06a;
  background: rgba(214, 176, 106, 0.14);
  border-radius: 5px;
  padding: 1px 5px;
}

.sidebar-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 18px;
  border-top: 1px solid var(--sidebar-border);
  font-size: 11px;
  color: var(--sidebar-muted);
}

.run-state {
  display: flex;
  align-items: center;
  gap: 7px;
}

.state-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #34d399;
  box-shadow: 0 0 0 3px rgba(52, 211, 153, 0.14);
}

.version {
  font-family: "SFMono-Regular", Menlo, Consolas, monospace;
  color: #64748b;
}
</style>
