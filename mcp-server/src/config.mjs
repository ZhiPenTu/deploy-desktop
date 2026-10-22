import fs from 'node:fs/promises';
import fsSync from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { DeployError } from './errors.mjs';
import { mcpConfigDir } from './paths.mjs';
import { formatJson, writePrivateFile } from './fs-atomic.mjs';
import { withFileLock } from './file-lock.mjs';
import { expandHome, isObject } from './util.mjs';

const moduleDir = path.dirname(fileURLToPath(import.meta.url));
export const PROJECT_ROOT = path.resolve(process.env.DEPLOY_DESKTOP_ROOT || path.join(moduleDir, '..', '..'));

export const DEFAULT_CONFIG = Object.freeze({
  version: 1,
  registries: {},
  hosts: {},
  update: {
    endpoint: '',
    allowInsecure: false
  }
});

export function configDir() {
  return mcpConfigDir();
}

export async function appVersion() {
  try {
    const content = await fs.readFile(path.join(PROJECT_ROOT, 'package.json'), 'utf8');
    return JSON.parse(content).version || '0.0.0';
  } catch {
    return process.env.DEPLOY_DESKTOP_VERSION || '0.0.0';
  }
}

export function configPath() {
  return expandHome(process.env.DEPLOY_DESKTOP_CONFIG_FILE || path.join(configDir(), 'mcp.json'));
}


function normalizeConfig(raw) {
  const config = {
    ...structuredClone(DEFAULT_CONFIG),
    ...(isObject(raw) ? raw : {})
  };
  config.version = 1;
  config.registries = isObject(config.registries) ? config.registries : {};
  config.hosts = isObject(config.hosts) ? config.hosts : {};
  config.update = isObject(config.update) ? { ...DEFAULT_CONFIG.update, ...config.update } : { ...DEFAULT_CONFIG.update };
  return config;
}

export async function loadConfig() {
  const file = configPath();
  try {
    const content = await fs.readFile(file, 'utf8');
    return { config: normalizeConfig(JSON.parse(content)), path: file };
  } catch (error) {
    if (error.code === 'ENOENT') return { config: normalizeConfig({}), path: file };
    if (error instanceof SyntaxError) {
      throw new DeployError(`MCP 配置文件不是有效 JSON: ${file}`, {
        code: 'CONFIG_INVALID',
        details: { path: file }
      });
    }
    throw new DeployError(`读取 MCP 配置失败: ${error.message}`, { code: 'CONFIG_READ_FAILED', details: { path: file } });
  }
}

export async function saveConfig(config) {
  const file = configPath();
  await withFileLock(file, () => writePrivateFile(file, formatJson(normalizeConfig(config))));
  return file;
}

export async function mutateConfig(mutator) {
  const file = configPath();
  return withFileLock(file, async () => {
    const { config } = await loadConfig();
    const next = structuredClone(config);
    const result = await mutator(next);
    await writePrivateFile(file, formatJson(normalizeConfig(next)));
    return { config: next, path: file, result };
  });
}

export function validateRegistry(registry) {
  if (!isObject(registry)) throw new DeployError('仓库配置必须是对象');
  const url = String(registry.url || '').trim();
  if (!url) throw new DeployError('仓库地址 url 不能为空');
  const withoutScheme = url.replace(/^https?:\/\//i, '').replace(/\/+$/, '');
  if (!withoutScheme || withoutScheme.includes('/') || withoutScheme.includes('@') ||
      /[^A-Za-z0-9.:[\]-]/.test(withoutScheme)) {
    throw new DeployError(`仓库地址无效: ${url}`, { hint: '只填写 registry 主机名，例如 registry.cn-hangzhou.aliyuncs.com' });
  }
  if (registry.namespace && !/^[A-Za-z0-9._/-]+$/.test(registry.namespace)) {
    throw new DeployError(`命名空间无效: ${registry.namespace}`);
  }
  if (registry.passwordEnv && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(registry.passwordEnv)) {
    throw new DeployError(`passwordEnv 必须是合法的环境变量名: ${registry.passwordEnv}`);
  }
  return {
    name: String(registry.name || '').trim(),
    url: withoutScheme.toLowerCase(),
    namespace: registry.namespace ? String(registry.namespace).replace(/^\/+|\/+$/g, '') : '',
    username: registry.username ? String(registry.username).trim() : '',
    passwordEnv: registry.passwordEnv ? String(registry.passwordEnv).trim() : '',
    insecure: Boolean(registry.insecure),
    isDefault: Boolean(registry.isDefault)
  };
}

export function validateHost(host) {
  if (!isObject(host)) throw new DeployError('主机配置必须是对象');
  const address = String(host.host || '').trim();
  if (!address) throw new DeployError('主机地址 host 不能为空');
  if (/[\s'`;$&|<>]/.test(address)) throw new DeployError(`主机地址含非法字符: ${address}`);
  const port = Number(host.port || 22);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new DeployError(`SSH 端口无效: ${host.port}`);
  const strictHostKeyChecking = String(host.strictHostKeyChecking || 'accept-new');
  if (!['yes', 'accept-new', 'no'].includes(strictHostKeyChecking)) {
    throw new DeployError(`strictHostKeyChecking 无效: ${host.strictHostKeyChecking}`, {
      hint: '可用值: yes, accept-new, no'
    });
  }
  return {
    name: String(host.name || '').trim(),
    host: address,
    port,
    user: String(host.user || 'root').trim() || 'root',
    keyPath: host.keyPath ? String(host.keyPath) : '',
    installDir: host.installDir ? String(host.installDir) : '',
    deployDir: host.deployDir ? String(host.deployDir) : '',
    bundleDir: host.bundleDir ? String(host.bundleDir) : '',
    strictHostKeyChecking
  };
}

export function ensureDefaultRegistry(config, name) {
  const registries = Object.values(config.registries || {});
  if (registries.some((registry) => registry.isDefault && registry.name !== name)) {
    for (const registry of registries) {
      if (registry.isDefault && registry.name !== name) registry.isDefault = false;
    }
  }
}

export function fileModeIsPrivate(file) {
  try {
    const stat = fsSync.statSync(file);
    if (process.platform === 'win32') return true;
    return (stat.mode & 0o077) === 0;
  } catch {
    return false;
  }
}
