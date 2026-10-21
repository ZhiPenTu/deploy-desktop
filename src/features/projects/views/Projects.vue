<template>
  <div class="page-view projects-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">项目管理</h2>
        <p class="page-subtitle">每个项目对应一套部署目标，切换后部署、监控、日志与数据库都使用当前项目</p>
      </div>
      <div class="header-actions">
        <el-button @click="$emit('go-hosts')">管理主机</el-button>
      </div>
    </div>

    <ProjectManager />
  </div>
</template>

<script setup>
import ProjectManager from '../components/ProjectManager.vue';

defineEmits(['go-hosts']);
</script>
