<template>
  <div class="page-view projects-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">项目管理</h2>
        <p class="page-subtitle">每个项目对应一套部署目标，选择「部署」进入部署流程</p>
      </div>
      <div class="header-actions">
        <el-button @click="$emit('go-hosts')">管理主机</el-button>
      </div>
    </div>

    <ProjectManager @deploy="$emit('deploy', $event)" />
  </div>
</template>

<script setup>
import ProjectManager from '../components/ProjectManager.vue';

defineEmits(['go-hosts', 'deploy']);
</script>
