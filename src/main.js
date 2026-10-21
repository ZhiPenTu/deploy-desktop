import { createApp } from 'vue';
import ElementPlus from 'element-plus';
import 'element-plus/dist/index.css';
import * as ElementPlusIconsVue from '@element-plus/icons-vue';

import App from './App.vue';
import { initProjects } from './config/project.js';
import { initConnectionInventory, syncConnectionFromProject } from './features/connections/state.js';
import './style.css';

async function bootstrap() {
  await initProjects();
  syncConnectionFromProject();
  await initConnectionInventory();

  const app = createApp(App);

  for (const [key, component] of Object.entries(ElementPlusIconsVue)) {
    app.component(key, component);
  }

  app.use(ElementPlus);
  app.mount('#app');
}

bootstrap();
