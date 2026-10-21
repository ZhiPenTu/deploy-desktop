import { reactive } from 'vue';
import {
  projectState,
  bindProjectBundle,
  refreshProjectContext,
  switchProject
} from '../../config/project.js';
import { listConnectionInventory, setActiveManagedHost } from '../../api/tauri.js';

/**
 * 连接信息全部由当前项目派生：项目决定主机与部署目录，
 * 交付包决定 Compose 位置，页面不再直接读取本机部署配置。
 */
export const connection = reactive({
  mode: 'remote',
  projectId: '',
  bundleDir: '',
  mediaDir: '',
  deployDir: '',
  installDir: '',
  remoteHost: '',
  remotePort: 22,
  remoteUser: 'root',
  remotePassword: '',
  remoteKey: '',
  remoteCredentialId: '',
  activeHostId: ''
});

export const connectionInventory = reactive({
  credentials: [],
  hosts: [],
  activeHostId: '',
  loading: false,
  error: ''
});

export function credentialById(id) {
  return connectionInventory.credentials.find((credential) => credential.id === id) || null;
}

export function hostById(id) {
  return connectionInventory.hosts.find((host) => host.id === id) || null;
}

export function isLocalHost(host) {
  return host?.kind === 'local';
}

export function credentialLabel(id) {
  const credential = credentialById(id);
  if (!credential) return '凭证不可用';
  return `${credential.name} · ${credential.username}`;
}

function clearHost(connectionState) {
  connectionState.activeHostId = '';
  connectionState.mode = 'local';
  connectionState.remoteHost = '';
  connectionState.remotePort = 22;
  connectionState.remoteUser = 'root';
  connectionState.remoteKey = '';
  connectionState.remoteCredentialId = '';
  connectionState.remotePassword = '';
}

function applyHost(hostId) {
  const host = hostById(hostId);
  if (!host) {
    clearHost(connection);
    return;
  }
  connection.activeHostId = host.id;
  if (isLocalHost(host)) {
    clearHost(connection);
    connection.activeHostId = host.id;
    return;
  }
  const credential = credentialById(host.credentialId);
  connection.mode = 'remote';
  connection.remoteHost = host.address;
  connection.remotePort = host.port || 22;
  connection.remoteUser = credential?.username || connection.remoteUser || 'root';
  connection.remoteKey = credential?.privateKeyPath || '';
  connection.remoteCredentialId = host.credentialId;
  connection.remotePassword = '';
}

/** 按当前项目刷新连接：主机、部署目录与交付包目录。 */
export function syncConnectionFromProject() {
  const project = projectState.project;
  if (!project?.id) {
    connection.projectId = '';
    connection.bundleDir = '';
    connection.mediaDir = '';
    connection.deployDir = '';
    connection.installDir = '';
    clearHost(connection);
    return;
  }
  connection.projectId = project.id;
  connection.installDir = project.installDir || '';
  connection.deployDir = projectState.deployDir || project.installDir || '';
  connection.bundleDir = project.lastBundleDir || '';
  connection.mediaDir = project.lastBundleDir || '';
  applyHost(project.hostId);
}

export function applyConnectionInventory(inventory) {
  connectionInventory.credentials = Array.isArray(inventory?.credentials)
    ? inventory.credentials
    : [];
  connectionInventory.hosts = Array.isArray(inventory?.hosts) ? inventory.hosts : [];
  connectionInventory.activeHostId = inventory?.activeHostId || '';
  connectionInventory.error = '';

  applyHost(projectState.project?.hostId || '');
}

export async function refreshConnectionInventory() {
  connectionInventory.loading = true;
  try {
    const inventory = await listConnectionInventory();
    applyConnectionInventory(inventory);
    return inventory;
  } catch (error) {
    connectionInventory.error = String(error);
    throw error;
  } finally {
    connectionInventory.loading = false;
  }
}

export async function initConnectionInventory() {
  try {
    await refreshConnectionInventory();
  } catch (error) {
    console.warn('读取凭证与主机配置失败，继续使用当前连接设置', error);
  }
}

/** 切换当前项目，并让所有页面跟随新的主机与部署目录。 */
export async function switchActiveProject(id) {
  const context = await switchProject(id);
  syncConnectionFromProject();
  await refreshConnectionInventory();
  return context;
}

/** 项目设置变化后重新读取项目与连接，不改变交付包绑定。 */
export async function reloadProjectState() {
  await refreshProjectContext();
  syncConnectionFromProject();
  return projectState;
}

/** 将交付包绑定到当前项目，供部署向导与制品中心共用。 */
export async function applyProjectBundle(bundleDir, version) {
  await bindProjectBundle(bundleDir, version);
  syncConnectionFromProject();
  return projectState.bundle;
}

export async function useManagedHost(id) {
  const inventory = await setActiveManagedHost(id);
  applyConnectionInventory(inventory);
  return inventory;
}
