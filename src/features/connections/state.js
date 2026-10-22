import { reactive } from 'vue';
import {
  projectState,
  bindProjectBundle,
  refreshProjectContext,
  switchProject
} from '../../config/project.js';
import { listConnectionInventory, setActiveManagedHost } from '../../api/tauri.js';
import { errorText } from '../../shared/errors.js';

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

/**
 * 连接信息按“整体赋值”写入：先清空再回填会让 activeHostId 出现一次瞬时变化，
 * 监听连接状态的页面（如部署向导）会误判成切换了目标。这里一次性算出目标值再写入。
 */
function assignHost(connectionState, values) {
  Object.assign(connectionState, values);
}

function applyHost(hostId) {
  const host = hostById(hostId);
  if (!host) {
    assignHost(connection, {
      activeHostId: '',
      mode: 'local',
      remoteHost: '',
      remotePort: 22,
      remoteUser: 'root',
      remoteKey: '',
      remoteCredentialId: '',
      remotePassword: ''
    });
    return;
  }
  if (isLocalHost(host)) {
    assignHost(connection, {
      activeHostId: host.id,
      mode: 'local',
      remoteHost: '',
      remotePort: 22,
      remoteUser: 'root',
      remoteKey: '',
      remoteCredentialId: '',
      remotePassword: ''
    });
    return;
  }
  const credential = credentialById(host.credentialId);
  assignHost(connection, {
    activeHostId: host.id,
    mode: 'remote',
    remoteHost: host.address,
    remotePort: host.port || 22,
    remoteUser: credential?.username || connection.remoteUser || 'root',
    remoteKey: credential?.privateKeyPath || '',
    remoteCredentialId: host.credentialId,
    remotePassword: ''
  });
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
    applyHost('');
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
    connectionInventory.error = errorText(error);
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
