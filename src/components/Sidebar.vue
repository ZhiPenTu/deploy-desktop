<template>
  <aside class="app-sidebar">
    <div class="nav-section">
      <div class="nav-label">工作台</div>
      <nav class="nav-menu">
        <button
          v-for="item in navItems"
          :key="item.key"
          type="button"
          class="nav-item"
          :class="{ active: currentTab === item.key }"
          @click="$emit('update:tab', item.key)"
        >
          <span class="nav-icon">
            <el-icon :size="18"><component :is="item.icon" /></el-icon>
          </span>
          <span class="nav-text">
            <span class="title">{{ item.title }}</span>
            <span class="desc">{{ item.desc }}</span>
          </span>
        </button>
      </nav>
    </div>

    <div class="sidebar-footer">
      <div class="footer-card">
        <div class="footer-title">部署工具</div>
        <div class="footer-meta">v{{ appVersion }} · 离线版</div>
        <div class="footer-meta">清单驱动部署</div>
      </div>
    </div>
  </aside>
</template>

<script setup>
import { Promotion, Odometer, DataBoard, Document, UploadFilled, Files, Connection } from '@element-plus/icons-vue';
import { defaultCluster } from '../config/cluster.js';

const appVersion = __APP_VERSION__;

defineProps({
  currentTab: {
    type: String,
    default: 'wizard'
  }
});

defineEmits(['update:tab']);

const navItems = [
  { key: 'wizard', icon: Promotion, title: '部署向导', desc: '全自动离线部署流程' },
  { key: 'env', icon: Odometer, title: '系统自检', desc: 'CPU / 内存 / 端口 / Docker' },
  { key: 'dashboard', icon: DataBoard, title: '集群监控', desc: `${defaultCluster.services.length} 个核心容器健康看板` },
  { key: 'update', icon: UploadFilled, title: '服务更新', desc: '增量更新已部署服务' },
  { key: 'images', icon: Files, title: '镜像包管理', desc: '离线镜像包上传与导入' },
  { key: 'registry', icon: Connection, title: '镜像仓库', desc: '清单拉取与离线导出' },
  { key: 'logs', icon: Document, title: '日志诊断', desc: '各容器运行日志流' },
];
</script>

<style scoped>
.app-sidebar {
  width: 232px;
  flex: none;
  background: rgba(15, 23, 36, 0.76);
  border-right: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}

.nav-section {
  padding: 20px 12px 0;
}

.nav-label {
  padding: 0 12px;
  margin-bottom: 10px;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.14em;
  color: #5f6f86;
}

.nav-menu {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.nav-item {
  position: relative;
  width: 100%;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition: background-color 0.18s ease, border-color 0.18s ease, color 0.18s ease;
}

.nav-item:hover {
  background: rgba(255, 255, 255, 0.035);
  color: var(--text-secondary);
}

.nav-item.active {
  background: rgba(79, 140, 255, 0.11);
  border-color: rgba(79, 140, 255, 0.2);
  color: var(--text-main);
}

.nav-item.active::before {
  content: "";
  position: absolute;
  left: -13px;
  top: 12px;
  bottom: 12px;
  width: 2px;
  border-radius: 2px;
  background: var(--primary-color);
}

.nav-icon {
  width: 34px;
  height: 34px;
  border-radius: 9px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
  background: rgba(255, 255, 255, 0.035);
  transition: background-color 0.18s ease, color 0.18s ease;
}

.nav-item.active .nav-icon {
  background: rgba(79, 140, 255, 0.16);
  color: #8ab6ff;
}

.nav-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.nav-text .title {
  font-size: 13px;
  font-weight: 500;
  line-height: 1.25;
}

.nav-text .desc {
  margin-top: 3px;
  font-size: 10px;
  line-height: 1.3;
  color: #6d7d93;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.sidebar-footer {
  padding: 16px;
  border-top: 1px solid var(--border-color);
}

.footer-card {
  padding: 12px 14px;
  border-radius: 10px;
  border: 1px solid rgba(255, 255, 255, 0.045);
  background: rgba(0, 0, 0, 0.16);
}

.footer-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
}

.footer-meta {
  margin-top: 4px;
  font-size: 10px;
  color: #64748b;
}
</style>
