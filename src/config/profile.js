import { reactive } from 'vue';
import { getDeploymentProfile } from '../api/tauri.js';
import exampleProfile from '../../profile.example.json';

function normalize(profile) {
  const source = profile && typeof profile === 'object' ? profile : {};
  return {
    name: source.name || '',
    containerPrefix: source.containerPrefix || '',
    bundleDir: source.bundleDir || '',
    mediaDir: source.mediaDir || source.bundleDir || '',
    deployDir: source.deployDir || '',
    installDir: source.installDir || '',
    backendArtifact: source.backendArtifact || '',
    database: source.database || {},
    containers: Array.isArray(source.containers) ? source.containers : [],
    images: Array.isArray(source.images) ? source.images : []
  };
}

export const profileState = reactive(normalize(exampleProfile));

function applyProfile(profile) {
  Object.assign(profileState, normalize(profile));
}

export async function initProfile() {
  try {
    const profile = await getDeploymentProfile();
    if (profile) {
      applyProfile(profile);
    }
  } catch (error) {
    console.warn('读取部署配置失败，继续使用示例配置', error);
  }
  return profileState;
}
