<template>
  <div class="app-layout" :class="{ 'platform-macos': isMacDesktop }">
    <Sidebar
      :current-tab="currentTab"
      @update:tab="navigateTab"
    />

    <div class="workspace">
      <Header
        :page-title="currentPage.title"
        :group="currentPage.group"
        :deploy-mode="connection.mode"
        :remote-host="connection.remoteHost"
        @quick-check="handleQuickCheck"
      />

      <main class="content-panel">
        <Projects
          v-if="currentTab === 'projects'"
          @go-hosts="navigateTab('hosts')"
          @deploy="handleDeployProject"
        />

        <Credentials
          v-if="currentTab === 'credentials'"
          @go-hosts="currentTab = 'hosts'"
        />

        <Hosts
          v-else-if="currentTab === 'hosts'"
          @view-status="viewHostStatus"
          @go-credentials="currentTab = 'credentials'"
        />

        <InstallWizard
          v-else-if="currentTab === 'wizard'"
          :connection="connection"
          @busy-change="wizardBusy = $event"
          @go-dashboard="navigateTab('dashboard')"
          @go-images="navigateTab('images')"
          @go-artifacts="navigateTab('artifacts')"
          @go-projects="navigateTab('projects')"
        />

        <EnvCheck
          v-else-if="currentTab === 'host-status'"
          :host="selectedStatusHost"
          :connection="connection"
          @go-hosts="navigateTab('hosts')"
        />

        <Dashboard
          v-else-if="currentTab === 'dashboard'"
          :connection="connection"
          @view-log="handleViewLog"
          @go-hosts="navigateTab('hosts')"
        />

        <ArtifactCenter
          v-else-if="currentTab === 'artifacts'"
          :connection="connection"
          @go-wizard="currentTab = 'wizard'"
          @go-settings="openSettings"
        />

        <ImageManager
          v-else-if="currentTab === 'images'"
          :connection="connection"
          @go-settings="openSettings"
        />

        <McpService v-else-if="currentTab === 'mcp'" />

        <Settings
          v-else-if="currentTab === 'settings'"
          :connection="connection"
          :initial-section="settingsSection"
          @go-artifacts="currentTab = 'artifacts'"
          @go-images="currentTab = 'images'"
        />

        <LogViewer
          v-else-if="currentTab === 'logs'"
          :connection="connection"
          :initial-container="activeLogContainer"
        />
      </main>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { ElMessage } from 'element-plus';
import { isDesktopApp } from './api/tauri.js';
import { projectState } from './config/project.js';
import {
  connection,
  hostById,
  switchActiveProject
} from './features/connections/state.js';
import Header from './components/layout/Header.vue';
import Sidebar from './components/layout/Sidebar.vue';
import Projects from './features/projects/views/Projects.vue';
import Credentials from './features/connections/views/Credentials.vue';
import Hosts from './features/connections/views/Hosts.vue';
import InstallWizard from './features/deployment/views/InstallWizard.vue';
import EnvCheck from './features/monitoring/views/EnvCheck.vue';
import Dashboard from './features/monitoring/views/Dashboard.vue';
import ImageManager from './features/images/views/ImageManager.vue';
import ArtifactCenter from './features/artifacts/views/ArtifactCenter.vue';
import Settings from './features/settings/views/Settings.vue';
import McpService from './features/mcp/views/McpService.vue';
import LogViewer from './features/monitoring/views/LogViewer.vue';


const pages = {
  projects: { title: '项目管理', group: '项目' },
  credentials: { title: '凭证管理', group: '连接' },
  hosts: { title: '主机管理', group: '连接' },
  wizard: { title: '部署向导', group: '部署' },
  'host-status': { title: '主机状态', group: '连接' },
  dashboard: { title: '集群监控', group: '运行' },
  logs: { title: '日志诊断', group: '运行' },
  artifacts: { title: '制品中心', group: '资源' },
  images: { title: '镜像与离线包', group: '资源' },
  mcp: { title: 'MCP 服务', group: '高级' },
  settings: { title: '设置', group: '高级' }
};

const currentTab = ref('wizard');
const settingsSection = ref('artifacts');
function openSettings(section) {
  settingsSection.value = section || 'artifacts';
  currentTab.value = 'settings';
}
const isMacDesktop = isDesktopApp() && /mac os x/i.test(navigator.userAgent);
const wizardBusy = ref(false);
const selectedStatusHostId = ref('');
const selectedStatusHost = computed(() => hostById(selectedStatusHostId.value));
function navigateTab(tab) {
  if (wizardBusy.value) { ElMessage.warning('当前操作正在进行，请等待完成后切换页面。'); return; }
  currentTab.value = tab;
}
const activeLogContainer = ref('');
const currentPage = computed(() => pages[currentTab.value] || pages.wizard);

const handleViewLog = (containerName) => {
  activeLogContainer.value = containerName;
  currentTab.value = 'logs';
};

async function handleDeployProject(id) {
  if (!id || wizardBusy.value) return;
  try {
    if (projectState.project?.id !== id) await switchActiveProject(id);
    activeLogContainer.value = '';
    navigateTab('wizard');
  } catch (error) {
    ElMessage.error(String(error));
  }
}

const handleQuickCheck = () => {
  const targetHost = hostById(selectedStatusHostId.value) || hostById(connection.activeHostId);
  selectedStatusHostId.value = targetHost?.id || '';
  navigateTab('host-status');
};

function viewHostStatus(host) {
  selectedStatusHostId.value = host?.id || '';
  navigateTab('host-status');
}
</script>

<style scoped>
.app-layout {
  display: flex;
  height: 100vh;
  width: 100vw;
  background: var(--app-bg);
  overflow: hidden;
}

.workspace {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.content-panel {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  background: var(--app-bg);
}
</style>
