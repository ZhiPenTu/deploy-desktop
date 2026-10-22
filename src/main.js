import { createApp } from 'vue';
import ElementPlus from 'element-plus';
import 'element-plus/dist/index.css';

import App from './App.vue';
import { reportClientEvent } from './api/tauri.js';
import { initProjects } from './config/project.js';
import { initConnectionInventory, syncConnectionFromProject } from './features/connections/state.js';
import './style.css';

function describeError(value) {
  if (!value) return '未知错误';
  if (typeof value === 'string') return value;
  return String(value.message || value.reason || value);
}

let lastFrontendError = '';
function reportFrontendError(message) {
  const text = describeError(message).slice(0, 2000);
  if (!text || text === lastFrontendError) return;
  lastFrontendError = text;
  reportClientEvent('error', 'frontend', text).catch(() => {});
}

window.addEventListener('error', (event) => {
  const location = event.filename ? ` (${event.filename}:${event.lineno || 0})` : '';
  reportFrontendError(`${event.message || '页面脚本执行失败'}${location}`);
});

window.addEventListener('unhandledrejection', (event) => {
  reportFrontendError(event.reason);
});

async function bootstrap() {
  await initProjects();
  syncConnectionFromProject();
  await initConnectionInventory();

  const app = createApp(App);

  app.use(ElementPlus);
  app.mount('#app');
}

bootstrap().catch((error) => {
  reportFrontendError(error);
  console.error(error);
});
