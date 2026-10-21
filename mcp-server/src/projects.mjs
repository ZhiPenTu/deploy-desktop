import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { DeployError } from './errors.mjs';
import { Bundle } from './bundle.mjs';
import { appVersion } from './config.mjs';
import { expandHome, isObject } from './util.mjs';

export const PROJECTS_FILE = 'projects.json';
export const CONNECTIONS_FILE = 'connections.json';
export const DEPLOYED_DIR = '.deploy-desktop';
export const DEPLOYED_FILE = 'deployed.json';

export function appConfigDir() {
  const override = process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
  if (override) return expandHome(override);
  const home = os.homedir();
  if (process.platform === 'darwin') {
    return path.join(home, 'Library', 'Application Support', 'com.deploydesktop.app');
  }
  if (process.platform === 'win32') {
    const appData = process.env.APPDATA || path.join(home, 'AppData', 'Roaming');
    return path.join(appData, 'com.deploydesktop.app');
  }
  return path.join(process.env.XDG_CONFIG_HOME || path.join(home, '.config'), 'com.deploydesktop.app');
}

async function readJson(file) {
  try {
    return JSON.parse(await fs.readFile(file, 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw new DeployError(`读取配置失败：${file}`, {
      code: 'CONFIG_READ_FAILED',
      details: { path: file, error: error.message }
    });
  }
}

export async function loadProjectStore() {
  const dir = appConfigDir();
  const raw = await readJson(path.join(dir, PROJECTS_FILE));
  if (raw === null) {
    return { schemaVersion: 1, activeProjectId: '', projects: [], path: path.join(dir, PROJECTS_FILE) };
  }
  if (!isObject(raw) || !Array.isArray(raw.projects)) {
    throw new DeployError(`项目配置格式不正确：${path.join(dir, PROJECTS_FILE)}`, { code: 'PROJECTS_INVALID' });
  }
  return {
    schemaVersion: Number(raw.schemaVersion) || 1,
    activeProjectId: String(raw.activeProjectId || ''),
    projects: raw.projects.filter(isObject).map((project) => ({
      id: String(project.id || ''),
      name: String(project.name || ''),
      product: String(project.product || ''),
      hostId: String(project.hostId || ''),
      installDir: String(project.installDir || ''),
      sourceId: String(project.sourceId || ''),
      lastVersion: String(project.lastVersion || ''),
      lastBundleDir: String(project.lastBundleDir || '')
    })),
    path: path.join(dir, PROJECTS_FILE)
  };
}

export async function activeProject() {
  const store = await loadProjectStore();
  if (store.projects.length === 0) {
    throw new DeployError('尚未创建项目，请先在部署舱的“设置 → 项目管理”中新建项目', {
      code: 'PROJECT_NOT_FOUND',
      hint: '一个项目对应一套部署目标：主机 + 部署目录 + 制品源。'
    });
  }
  const id = store.activeProjectId.trim();
  const project = id ? store.projects.find((item) => item.id === id) : store.projects[0];
  if (!project) {
    throw new DeployError(`当前项目不存在：${id}`, { code: 'PROJECT_NOT_FOUND', hint: '请刷新项目列表后重新选择。' });
  }
  return { project, store };
}

export async function activeBundle() {
  const { project, store } = await activeProject();
  const dir = project.lastBundleDir.trim();
  if (!dir) {
    throw new DeployError(`项目「${project.name}」尚未绑定交付包`, {
      code: 'BUNDLE_NOT_BOUND',
      hint: '请先在制品中心选择并绑定一个交付包。'
    });
  }
  const bundle = await Bundle.load(dir, { appVersion: await appVersion() });
  if (project.product && project.product !== bundle.product()) {
    throw new DeployError(`交付包属于「${bundle.product()}」，项目已绑定「${project.product}」`, {
      code: 'PRODUCT_MISMATCH'
    });
  }
  return { project, bundle, store };
}

export async function loadConnections() {
  const file = path.join(appConfigDir(), CONNECTIONS_FILE);
  const raw = await readJson(file);
  if (!raw) return { hosts: [], credentials: [], path: file };
  return {
    hosts: Array.isArray(raw.hosts) ? raw.hosts : [],
    credentials: Array.isArray(raw.credentials) ? raw.credentials : [],
    path: file
  };
}

export async function projectHost(project) {
  const { hosts, credentials } = await loadConnections();
  const host = hosts.find((item) => item.id === project?.hostId);
  if (!host) {
    throw new DeployError(`项目「${project?.name || ''}」绑定的主机不存在`, {
      code: 'HOST_NOT_FOUND',
      hint: '请在“设置 → 管理主机”中确认主机是否已被删除，并重新为项目选择主机。'
    });
  }
  if (String(host.kind || '').toLowerCase() === 'local') {
    throw new DeployError('当前项目使用本机部署，MCP 工具仅支持通过 SSH 操作远程主机', { code: 'HOST_LOCAL_UNSUPPORTED' });
  }
  const credential = credentials.find((item) => item.id === host.credentialId) || null;
  return {
    id: host.id,
    name: host.name || host.address || host.id,
    address: host.address || '',
    port: Number(host.port) || 22,
    kind: host.kind || 'ssh',
    credentialId: host.credentialId || '',
    user: credential?.username || 'root',
    keyPath: credential?.privateKeyPath || ''
  };
}

export function deployedRecordPath(installDir) {
  return path.join(installDir, DEPLOYED_DIR, DEPLOYED_FILE);
}

export async function readDeployedRecord(installDir) {
  const raw = await readJson(deployedRecordPath(installDir)).catch(() => null);
  if (!raw || !Array.isArray(raw.artifacts)) return null;
  return raw;
}
