import { reactive } from 'vue';
import { profileState } from './config/profile.js';
import { listConnectionInventory, setActiveManagedHost } from './api/tauri.js';

export const connection = reactive({
  mode: 'remote',
  bundleDir: profileState.bundleDir,
  mediaDir: profileState.mediaDir || profileState.bundleDir,
  deployDir: profileState.deployDir,
  installDir: profileState.installDir,
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

export function syncConnectionFromProfile() {
  if (profileState.bundleDir) {
    connection.bundleDir = profileState.bundleDir;
  }
  if (profileState.mediaDir) {
    connection.mediaDir = profileState.mediaDir;
  }
  if (profileState.deployDir) {
    connection.deployDir = profileState.deployDir;
  }
  if (profileState.installDir) {
    connection.installDir = profileState.installDir;
  }
}

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

function applyManagedHostToConnection(host) {
  if (!host) return;
  connection.activeHostId = host.id;
  if (isLocalHost(host)) {
    connection.mode = 'local';
    connection.remoteHost = '';
    connection.remotePort = 22;
    connection.remoteUser = 'root';
    connection.remoteKey = '';
    connection.remoteCredentialId = '';
    connection.remotePassword = '';
  } else {
    const credential = credentialById(host.credentialId);
    connection.mode = 'remote';
    connection.remoteHost = host.address;
    connection.remotePort = host.port || 22;
    connection.remoteUser = credential?.username || connection.remoteUser || 'root';
    connection.remoteKey = credential?.privateKeyPath || '';
    connection.remoteCredentialId = host.credentialId;
    connection.remotePassword = '';
  }
  if (host.deployDir) {
    connection.deployDir = host.deployDir;
    connection.installDir = host.deployDir;
  } else {
    if (profileState.deployDir) connection.deployDir = profileState.deployDir;
    if (profileState.installDir) connection.installDir = profileState.installDir;
  }
}

export function applyConnectionInventory(inventory) {
  connectionInventory.credentials = Array.isArray(inventory?.credentials)
    ? inventory.credentials
    : [];
  connectionInventory.hosts = Array.isArray(inventory?.hosts) ? inventory.hosts : [];
  connectionInventory.activeHostId = inventory?.activeHostId || '';
  connectionInventory.error = '';

  const activeHost = hostById(connectionInventory.activeHostId);
  if (activeHost) {
    applyManagedHostToConnection(activeHost);
  } else {
    connection.activeHostId = '';
    connection.mode = 'local';
    connection.remoteHost = '';
    connection.remotePort = 22;
    connection.remoteUser = 'root';
    connection.remotePassword = '';
    connection.remoteKey = '';
    connection.remoteCredentialId = '';
  }
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

export async function useManagedHost(id) {
  const inventory = await setActiveManagedHost(id);
  applyConnectionInventory(inventory);
  return inventory;
}
