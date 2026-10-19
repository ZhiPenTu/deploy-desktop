import fs from 'node:fs/promises';
import path from 'node:path';
import { PROJECT_ROOT, loadConfig } from './config.mjs';
import { DeployError } from './errors.mjs';
import { expandHome, isObject } from './util.mjs';

const PROFILE_KEYS = [
  'name',
  'containerPrefix',
  'bundleDir',
  'deployDir',
  'installDir',
  'backendArtifact',
  'components',
  'database',
  'containers',
  'images'
];

function requireText(value, field) {
  if (typeof value !== 'string' || !value.trim()) {
    throw new DeployError(`profile.${field} 不能为空`);
  }
  if (/[\r\n]/.test(value)) {
    throw new DeployError(`profile.${field} 不能包含换行`);
  }
  return value.trim();
}

export function validateProfile(profile) {
  if (!isObject(profile)) throw new DeployError('profile 必须是 JSON 对象');
  const result = {
    name: requireText(profile.name, 'name'),
    containerPrefix: requireText(profile.containerPrefix, 'containerPrefix'),
    bundleDir: requireText(profile.bundleDir, 'bundleDir'),
    deployDir: requireText(profile.deployDir, 'deployDir'),
    installDir: requireText(profile.installDir, 'installDir'),
    backendArtifact: requireText(profile.backendArtifact, 'backendArtifact'),
    components: { backend: [], gateway: [], frontend: [] },
    database: null,
    containers: [],
    images: []
  };

  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(result.containerPrefix)) {
    throw new DeployError('profile.containerPrefix 只能包含字母、数字、点、下划线和短横线');
  }
  if (!/^[A-Za-z0-9._-]+$/.test(result.backendArtifact)) {
    throw new DeployError('profile.backendArtifact 只能包含字母、数字、点、下划线和短横线');
  }

  if (profile.components !== undefined) {
    if (!isObject(profile.components)) throw new DeployError('profile.components 必须是对象');
    for (const key of ['backend', 'gateway', 'frontend']) {
      const value = profile.components[key];
      if (value === undefined) continue;
      if (!Array.isArray(value)) throw new DeployError(`profile.components.${key} 必须是数组`);
      for (const [index, serviceKey] of value.entries()) {
        if (typeof serviceKey !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(serviceKey)) {
          throw new DeployError(`profile.components.${key}[${index}] 不是合法的服务键`);
        }
      }
      result.components[key] = [...value];
    }
  }

  if (!isObject(profile.database)) throw new DeployError('profile.database 必须是对象');
  const database = profile.database;
  const dbPort = Number(database.port);
  if (!Number.isInteger(dbPort) || dbPort < 1 || dbPort > 65535) {
    throw new DeployError('profile.database.port 必须是 1-65535 的整数');
  }
  result.database = {
    serviceKey: requireText(database.serviceKey, 'database.serviceKey'),
    name: requireText(database.name, 'database.name'),
    client: requireText(database.client, 'database.client'),
    port: dbPort
  };
  if (!/^[A-Za-z0-9_]+$/.test(result.database.name)) {
    throw new DeployError('profile.database.name 只能包含字母、数字和下划线');
  }
  if (!/^[A-Za-z0-9._-]+$/.test(result.database.client)) {
    throw new DeployError('profile.database.client 只能包含字母、数字、点、下划线和短横线');
  }

  if (!Array.isArray(profile.containers) || profile.containers.length === 0) {
    throw new DeployError('profile.containers 必须是非空数组');
  }
  const serviceKeys = new Set();
  for (const [index, container] of profile.containers.entries()) {
    if (!isObject(container)) throw new DeployError(`profile.containers[${index}] 必须是对象`);
    const serviceKey = requireText(container.serviceKey, `containers[${index}].serviceKey`);
    if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(serviceKey)) {
      throw new DeployError(`profile.containers[${index}].serviceKey 只能包含字母、数字、点、下划线和短横线`);
    }
    if (serviceKeys.has(serviceKey)) throw new DeployError(`profile.containers serviceKey 重复: ${serviceKey}`);
    serviceKeys.add(serviceKey);
    const protocol = String(container.protocol || 'TCP').toUpperCase();
    if (!['TCP', 'UDP'].includes(protocol)) {
      throw new DeployError(`profile.containers[${index}].protocol 必须是 TCP 或 UDP`);
    }
    const port = container.port === null || container.port === undefined ? null : Number(container.port);
    if (port !== null && (!Number.isInteger(port) || port < 1 || port > 65535)) {
      throw new DeployError(`profile.containers[${index}].port 必须是 1-65535 的整数或 null`);
    }
    result.containers.push({
      serviceKey,
      label: requireText(container.label, `containers[${index}].label`),
      port,
      protocol,
      description: String(container.description || '')
    });
  }
  if (!serviceKeys.has(result.database.serviceKey)) {
    throw new DeployError(`profile.database.serviceKey 未出现在 containers 中: ${result.database.serviceKey}`);
  }

  if (!Array.isArray(profile.images) || profile.images.length === 0) {
    throw new DeployError('profile.images 必须是非空数组');
  }
  const imageFiles = new Set();
  for (const [index, image] of profile.images.entries()) {
    if (!isObject(image)) throw new DeployError(`profile.images[${index}] 必须是对象`);
    const file = requireText(image.file, `images[${index}].file`);
    if (!/^[A-Za-z0-9._-]+$/.test(file)) {
      throw new DeployError(`profile.images[${index}].file 只能包含字母、数字、点、下划线和短横线`);
    }
    if (imageFiles.has(file)) throw new DeployError(`profile.images file 重复: ${file}`);
    imageFiles.add(file);
    result.images.push({
      file,
      imageRef: requireText(image.imageRef, `images[${index}].imageRef`)
    });
  }

  return result;
}

function userProfilePath() {
  const home = process.env.HOME || '';
  if (process.platform === 'darwin') {
    return path.join(home, 'Library', 'Application Support', 'com.deploydesktop.app', 'profile.json');
  }
  if (process.platform === 'win32') {
    const appData = process.env.APPDATA || path.join(home, 'AppData', 'Roaming');
    return path.join(appData, 'com.deploydesktop.app', 'profile.json');
  }
  return path.join(process.env.XDG_CONFIG_HOME || path.join(home, '.config'), 'com.deploydesktop.app', 'profile.json');
}

export async function resolveProfilePath() {
  const candidates = [];
  if (process.env.DEPLOY_DESKTOP_PROFILE) candidates.push(expandHome(process.env.DEPLOY_DESKTOP_PROFILE));
  const { config } = await loadConfig();
  if (config.profilePath) candidates.push(expandHome(config.profilePath));
  candidates.push(userProfilePath());
  candidates.push(path.join(PROJECT_ROOT, 'profile.json'));
  candidates.push(path.join(PROJECT_ROOT, 'profile.example.json'));
  for (const candidate of candidates) {
    try {
      await fs.access(candidate);
      return candidate;
    } catch {
      // continue
    }
  }
  throw new DeployError('未找到 profile.json，请设置 DEPLOY_DESKTOP_PROFILE 或 DEPLOY_DESKTOP_ROOT', {
    code: 'PROFILE_NOT_FOUND'
  });
}

export async function loadProfile() {
  const file = await resolveProfilePath();
  const content = await fs.readFile(file, 'utf8');
  let parsed;
  try {
    parsed = JSON.parse(content);
  } catch (error) {
    throw new DeployError(`profile.json 不是有效 JSON: ${file}`, {
      code: 'PROFILE_INVALID',
      details: { path: file, error: error.message }
    });
  }
  return { profile: validateProfile(parsed), source: file };
}

export async function appVersion() {
  try {
    const content = await fs.readFile(path.join(PROJECT_ROOT, 'package.json'), 'utf8');
    return JSON.parse(content).version || '0.0.0';
  } catch {
    return process.env.DEPLOY_DESKTOP_VERSION || '0.0.0';
  }
}

export async function updateProfile(patch, options = {}) {
  if (!isObject(patch)) throw new DeployError('profile patch 必须是对象');
  const unknown = Object.keys(patch).filter((key) => !PROFILE_KEYS.includes(key));
  if (unknown.length > 0) throw new DeployError(`不支持的 profile 字段: ${unknown.join(', ')}`);
  const { profile, source } = await loadProfile();
  const next = structuredClone(profile);
  for (const key of PROFILE_KEYS) {
    if (patch[key] !== undefined) next[key] = structuredClone(patch[key]);
  }
  const validated = validateProfile(next);
  const changed = PROFILE_KEYS.filter((key) => JSON.stringify(profile[key]) !== JSON.stringify(validated[key]));
  if (options.dryRun) {
    return { dryRun: true, source, changed, profile: validated };
  }
  if (options.confirm !== true) {
    throw new DeployError('修改部署配置需要显式确认：请传入 confirm=true', {
      code: 'CONFIRM_REQUIRED',
      hint: '如只想预览变更，请使用 dryRun=true。'
    });
  }
  const dir = path.dirname(source);
  const temp = path.join(dir, `.profile.${process.pid}.${Date.now()}.tmp`);
  await fs.writeFile(temp, `${JSON.stringify(validated, null, 2)}\n`, { mode: 0o644 });
  await fs.copyFile(source, `${source}.bak`).catch(() => {});
  await fs.rename(temp, source);
  return { dryRun: false, source, changed, profile: validated };
}

export function containerName(profile, serviceKey) {
  return `${profile.containerPrefix}-${serviceKey}`;
}

export function resolveContainer(profile, input) {
  const value = String(input || '').trim();
  if (!value) throw new DeployError('请填写服务名或容器名');
  if (value.startsWith(`${profile.containerPrefix}-`)) return value;
  const service = profile.containers.find((container) => container.serviceKey === value);
  if (!service) {
    throw new DeployError(`profile 中不存在服务: ${value}`, {
      hint: `可用服务: ${profile.containers.map((container) => container.serviceKey).join(', ')}`
    });
  }
  return containerName(profile, value);
}

export function databaseContainer(profile) {
  return containerName(profile, profile.database.serviceKey);
}

export function serviceKeysForComponent(profile, component) {
  const configured = profile.components?.[component];
  if (Array.isArray(configured) && configured.length > 0) {
    return [...configured];
  }
  if (component === 'backend') {
    return profile.containers.map((container) => container.serviceKey).filter((key) => key.startsWith('backend-java'));
  }
  if (component === 'gateway') {
    return profile.containers.map((container) => container.serviceKey).filter((key) => key.startsWith('gateway-'));
  }
  if (component === 'frontend') {
    return profile.containers.map((container) => container.serviceKey).filter((key) => key === 'frontend');
  }
  if (component === 'config') {
    return profile.containers.map((container) => container.serviceKey);
  }
  return [];
}
