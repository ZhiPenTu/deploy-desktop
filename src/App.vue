<template>
  <div class="app-layout">
    <Sidebar :current-tab="currentTab" @update:tab="currentTab = $event" />

    <div class="workspace">
      <Header
        :page-title="currentPage.title"
        :group="currentPage.group"
        :deploy-mode="connection.mode"
        :remote-host="connection.remoteHost"
        @quick-check="handleQuickCheck"
      />

      <main class="content-panel">
        <Credentials
          v-if="currentTab === 'credentials'"
          @go-hosts="currentTab = 'hosts'"
        />

        <Hosts
          v-else-if="currentTab === 'hosts'"
          @go-credentials="currentTab = 'credentials'"
        />

        <InstallWizard
          v-else-if="currentTab === 'wizard'"
          :connection="connection"
          @go-dashboard="currentTab = 'dashboard'"
          @go-images="currentTab = 'images'"
          @go-artifacts="currentTab = 'artifacts'"
          @go-hosts="currentTab = 'hosts'"
        />

        <EnvCheck
          v-else-if="currentTab === 'env'"
          :connection="connection"
        />

        <Dashboard
          v-else-if="currentTab === 'dashboard'"
          :connection="connection"
          @view-log="handleViewLog"
        />

        <ServiceUpdate
          v-else-if="currentTab === 'update'"
          :connection="connection"
          @go-dashboard="currentTab = 'dashboard'"
        />

        <ArtifactCenter
          v-else-if="currentTab === 'artifacts'"
          :connection="connection"
          @go-wizard="currentTab = 'wizard'"
          @go-update="currentTab = 'update'"
        />

        <ImageManager
          v-else-if="currentTab === 'images'"
          :connection="connection"
          @go-settings="currentTab = 'settings'"
        />

        <RegistryCenter
          v-else-if="currentTab === 'registry'"
          :connection="connection"
        />

        <Settings
          v-else-if="currentTab === 'settings'"
          :connection="connection"
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
import { connection } from './connection';
import Header from './components/Header.vue';
import Sidebar from './components/Sidebar.vue';
import Credentials from './views/Credentials.vue';
import Hosts from './views/Hosts.vue';
import InstallWizard from './views/InstallWizard.vue';
import EnvCheck from './views/EnvCheck.vue';
import Dashboard from './views/Dashboard.vue';
import ServiceUpdate from './views/ServiceUpdate.vue';
import ImageManager from './views/ImageManager.vue';
import ArtifactCenter from './views/ArtifactCenter.vue';
import RegistryCenter from './views/RegistryCenter.vue';
import Settings from './views/Settings.vue';
import LogViewer from './views/LogViewer.vue';
import { getDefaultContainer } from './config/cluster.js';

const pages = {
  credentials: { title: '凭证管理', group: '连接' },
  hosts: { title: '主机管理', group: '连接' },
  wizard: { title: '部署向导', group: '部署' },
  env: { title: '系统自检', group: '部署' },
  dashboard: { title: '集群监控', group: '运行' },
  update: { title: '服务更新', group: '运行' },
  logs: { title: '日志诊断', group: '运行' },
  artifacts: { title: '制品中心', group: '资源' },
  images: { title: '镜像与离线包', group: '资源' },
  registry: { title: '镜像仓库', group: '高级' },
  settings: { title: '设置', group: '高级' }
};

const currentTab = ref('wizard');
const activeLogContainer = ref(getDefaultContainer());
const currentPage = computed(() => pages[currentTab.value] || pages.wizard);

const handleViewLog = (containerName) => {
  activeLogContainer.value = containerName;
  currentTab.value = 'logs';
};

const handleQuickCheck = () => {
  currentTab.value = 'env';
};
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
