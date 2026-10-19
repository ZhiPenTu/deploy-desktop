<template>
  <div class="app-layout">
    <Header
      :deploy-mode="connection.mode"
      :remote-host="connection.remoteHost"
      @quick-check="handleQuickCheck"
    />

    <div class="main-body">
      <Sidebar :current-tab="currentTab" @update:tab="currentTab = $event" />

      <main class="content-panel">
        <InstallWizard
          v-if="currentTab === 'wizard'"
          :connection="connection"
          @go-dashboard="currentTab = 'dashboard'"
          @go-images="currentTab = 'images'"
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

        <ImageManager
          v-else-if="currentTab === 'images'"
          :connection="connection"
        />

        <RegistryCenter
          v-else-if="currentTab === 'registry'"
          :connection="connection"
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
import { ref } from 'vue';
import { connection } from './connection';
import Header from './components/Header.vue';
import Sidebar from './components/Sidebar.vue';
import InstallWizard from './views/InstallWizard.vue';
import EnvCheck from './views/EnvCheck.vue';
import Dashboard from './views/Dashboard.vue';
import ServiceUpdate from './views/ServiceUpdate.vue';
import ImageManager from './views/ImageManager.vue';
import RegistryCenter from './views/RegistryCenter.vue';
import LogViewer from './views/LogViewer.vue';
import { defaultContainer } from './config/cluster.js';

const currentTab = ref('wizard');
const activeLogContainer = ref(defaultContainer);

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
  flex-direction: column;
  height: 100vh;
  width: 100vw;
  background: transparent;
  overflow: hidden;
}

.main-body {
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.content-panel {
  flex: 1;
  min-width: 0;
  background: transparent;
  height: 100%;
  overflow: hidden;
}
</style>
