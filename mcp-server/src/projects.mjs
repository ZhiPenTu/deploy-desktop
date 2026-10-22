import path from 'node:path';
import { DeployError } from './errors.mjs';
import { Bundle } from './bundle.mjs';
import { appVersion } from './config.mjs';
import { appConfigDir } from './paths.mjs';
import { formatJson, readJsonFile, writePrivateFile } from './fs-atomic.mjs';
import { withFileLock } from './file-lock.mjs';
import { isObject } from './util.mjs';

export { appConfigDir };

export const PROJECTS_FILE = 'projects.json';
export const CONNECTIONS_FILE = 'connections.json';
export const DEPLOYED_DIR = '.deploy-desktop';
export const DEPLOYED_FILE = 'deployed.json';

let storeQueue = Promise.resolve();

async function readJson(file, label) {
  try {
    return await readJsonFile(file);
  } catch (error) {
    throw new DeployError(`读取${label}失败：${file}`, {
      code: 'CONFIG_READ_FAILED',
      details: { path: file, error: error.message }
    });
  }
}

/** 不加锁的写入实现；调用方必须持有 `file-lock`，避免嵌套加锁。 */
async function writeStoreFile(file, value) {
  await writePrivateFile(file, formatJson(value));
}

export function normalizeProjectStore(raw) {
  if (!isObject(raw) || !Array.isArray(raw.projects)) {
    throw new DeployError('项目配置格式不正确，请检查本机项目配置', { code: 'PROJECTS_INVALID' });
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
    }))
  };
}

export async function loadProjectStore() {
  const dir = appConfigDir();
  const file = path.join(dir, PROJECTS_FILE);
  const raw = await readJson(file, '项目配置');
  if (raw === null) {
    return { schemaVersion: 1, activeProjectId: '', projects: [], path: file };
  }
  try {
    return { ...normalizeProjectStore(raw), path: file };
  } catch (error) {
    throw new DeployError(`项目配置格式不正确：${file}`, {
      code: 'PROJECTS_INVALID',
      details: { path: file }
    });
  }
}

export async function saveProjectStore(store) {
  const file = path.join(appConfigDir(), PROJECTS_FILE);
  await withFileLock(file, () => writeStoreFile(file, normalizeProjectStore(store)));
  return file;
}

/**
 * 读取、修改、写回都在同一把跨进程锁内完成。
 *
 * 桌面端「设置 → 项目管理」会写同一份 `projects.json`，先锁后读再写，
 * 两边的修改才不会因为各自基于旧快照而互相覆盖。
 */
export function mutateProjectStore(mutator) {
  const run = storeQueue.then(async () => {
    const file = path.join(appConfigDir(), PROJECTS_FILE);
    return withFileLock(file, async () => {
      const store = await loadProjectStore();
      const next = structuredClone(store);
      const result = await mutator(next);
      await writeStoreFile(file, normalizeProjectStore(next));
      return { store: next, path: file, result };
    });
  });
  storeQueue = run.catch(() => {});
  return run;
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
  const raw = await readJson(file, '连接配置');
  if (!raw) return { hosts: [], credentials: [], path: file };
  return {
    hosts: Array.isArray(raw.hosts) ? raw.hosts : [],
    credentials: Array.isArray(raw.credentials) ? raw.credentials : [],
    path: file
  };
}

export function managedHosts(raw) {
  return (Array.isArray(raw?.hosts) ? raw.hosts : []).map((host) => ({
    id: String(host.id || ''),
    kind: String(host.kind || 'ssh'),
    name: String(host.name || ''),
    address: String(host.address || ''),
    port: Number(host.port) || 22,
    credentialId: String(host.credentialId || ''),
    notes: String(host.notes || '')
  }));
}

export async function resolveManagedHost(selector) {
  const value = String(selector || '').trim();
  if (!value) throw new DeployError('请选择部署主机', { code: 'HOST_REQUIRED' });
  const { hosts } = await loadConnections();
  const managed = managedHosts({ hosts });
  const byId = managed.filter((host) => host.id === value);
  if (byId.length === 1) return byId[0];
  const byName = managed.filter((host) => host.name === value);
  if (byName.length === 1) return byName[0];
  const byAddress = managed.filter((host) => host.address === value);
  if (byAddress.length === 1) return byAddress[0];
  if (byId.length > 1 || byName.length > 1 || byAddress.length > 1) {
    throw new DeployError(`主机「${value}」匹配到多个配置，请改用主机 ID`, { code: 'HOST_AMBIGUOUS' });
  }
  throw new DeployError(`主机不存在：${value}`, {
    code: 'HOST_NOT_FOUND',
    hint: '请先在连接设置中创建主机，再创建或编辑项目。'
  });
}

export function validateProjectName(raw) {
  const name = String(raw ?? '').trim();
  if (!name) throw new DeployError('请填写项目名称', { code: 'PROJECT_NAME_REQUIRED' });
  if ([...name].length > 60) throw new DeployError('项目名称不能超过 60 个字符', { code: 'PROJECT_NAME_TOO_LONG' });
  if (/[\r\n\0]/.test(name)) throw new DeployError('项目名称包含不支持的字符', { code: 'PROJECT_NAME_INVALID' });
  return name;
}

export function validateInstallDir(raw) {
  const value = String(raw ?? '').trim().replace(/\/+$/, '');
  if (!value) throw new DeployError('请填写部署目录', { code: 'INSTALL_DIR_REQUIRED' });
  if (/[\r\n\0]/.test(value)) throw new DeployError('部署目录包含不支持的字符', { code: 'INSTALL_DIR_INVALID' });
  const windowsAbsolute = /^[A-Za-z]:[\\/]/.test(value);
  if (!value.startsWith('/') && !windowsAbsolute) {
    throw new DeployError('部署目录必须填写绝对路径', { code: 'INSTALL_DIR_NOT_ABSOLUTE' });
  }
  if (value.split(/[\\/]/).some((part) => part === '..')) {
    throw new DeployError('部署目录不能包含上级目录', { code: 'INSTALL_DIR_PARENT' });
  }
  return value;
}

export function findProject(store, selector) {
  const value = String(selector || '').trim();
  if (!value) return null;
  const byId = store.projects.filter((project) => project.id === value);
  if (byId.length === 1) return byId[0];
  const byName = store.projects.filter((project) => project.name === value);
  if (byName.length === 1) return byName[0];
  if (byName.length > 1) {
    throw new DeployError(`项目「${value}」名称重复，请改用项目 ID`, { code: 'PROJECT_AMBIGUOUS' });
  }
  return null;
}

export function requireProject(store, selector) {
  const project = findProject(store, selector);
  if (!project) {
    throw new DeployError(`项目不存在：${selector || ''}`, {
      code: 'PROJECT_NOT_FOUND',
      hint: '请先查看项目列表，确认项目 ID 或名称。'
    });
  }
  return project;
}

export function nextProjectId(store) {
  const stamp = Date.now();
  let candidate = `p-${stamp}`;
  let index = 1;
  while (store.projects.some((project) => project.id === candidate)) {
    candidate = `p-${stamp}-${index}`;
    index += 1;
  }
  return candidate;
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
  const raw = await readJson(deployedRecordPath(installDir), '部署记录').catch(() => null);
  if (!raw || !Array.isArray(raw.artifacts)) return null;
  return raw;
}
