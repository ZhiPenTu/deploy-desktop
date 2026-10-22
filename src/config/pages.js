import {
  Box,
  Connection,
  Files,
  FolderOpened,
  Key,
  Monitor,
  Promotion,
  Setting
} from '@element-plus/icons-vue';

/**
 * 页面元数据只在这里维护一份。
 *
 * - `group` 决定侧边栏分组与页头分组；分组顺序按页面顺序首次出现决定。
 * - `hidden` 表示该页面只能从其他页面的操作进入，不在侧边栏出现。
 * - 新增页面时同时补齐 `pageOrder`，否则侧边栏会漏项。
 */
export const pages = {
  projects: { title: '项目管理', group: '项目', icon: FolderOpened },
  credentials: { title: '凭证管理', group: '连接', icon: Key },
  hosts: { title: '主机管理', group: '连接', icon: Monitor },
  wizard: { title: '部署向导', group: '部署', icon: Promotion },
  'host-status': { title: '主机状态', group: '连接', hidden: true },
  dashboard: { title: '集群监控', group: '运行', hidden: true },
  logs: { title: '日志诊断', group: '运行', hidden: true },
  artifacts: { title: '制品中心', group: '资源', icon: Box },
  images: { title: '镜像与离线包', group: '资源', icon: Files },
  mcp: { title: 'MCP 服务', group: '高级', icon: Connection },
  settings: { title: '设置', group: '高级', icon: Setting }
};

export const pageOrder = [
  'projects',
  'credentials',
  'hosts',
  'wizard',
  'host-status',
  'dashboard',
  'logs',
  'artifacts',
  'images',
  'mcp',
  'settings'
];

export const defaultPage = 'wizard';

export function pageOf(key) {
  return pages[key] || pages[defaultPage];
}

/** 侧边栏分组：跳过只能通过操作进入的页面。 */
export function navGroups() {
  const groups = new Map();
  for (const key of pageOrder) {
    const page = pages[key];
    if (!page || page.hidden) continue;
    if (!groups.has(page.group)) groups.set(page.group, []);
    groups.get(page.group).push({
      key,
      title: page.title,
      icon: page.icon,
      badge: page.badge
    });
  }
  return [...groups].map(([label, items]) => ({ label, items }));
}
