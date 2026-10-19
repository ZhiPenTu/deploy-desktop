import { createApp } from 'vue';
import ElementPlus from 'element-plus';
import 'element-plus/dist/index.css';
import * as ElementPlusIconsVue from '@element-plus/icons-vue';

import App from './App.vue';
import { initProfile } from './config/profile.js';
import { syncConnectionFromProfile } from './connection.js';
import './style.css';

async function bootstrap() {
  await initProfile();
  syncConnectionFromProfile();

  const app = createApp(App);

  for (const [key, component] of Object.entries(ElementPlusIconsVue)) {
    app.component(key, component);
  }

  app.use(ElementPlus);
  app.mount('#app');
}

bootstrap();
