import crypto from 'node:crypto';
import { loadConfig } from './config.mjs';
import { DeployError } from './errors.mjs';
import { requireBinary, run } from './exec.mjs';
import { normalizeRegistry } from './manifest.mjs';
import { buildSshArgs, explainSshFailure, runRemote } from './ssh.mjs';
import { shellQuote } from './util.mjs';

const MANIFEST_ACCEPT = [
  'application/vnd.oci.image.index.v1+json',
  'application/vnd.docker.distribution.manifest.list.v2+json',
  'application/vnd.oci.image.manifest.v1+json',
  'application/vnd.docker.distribution.manifest.v2+json'
].join(', ');

function baseUrl(registry, insecure) {
  return `${insecure ? 'http' : 'https'}://${registry}`;
}

function basicAuth(username, password) {
  return `Basic ${Buffer.from(`${username}:${password}`).toString('base64')}`;
}

export function parseWwwAuthenticate(header) {
  if (!header) return null;
  const match = String(header).match(/^\s*(\w+)\s+(.*)$/);
  if (!match) return null;
  const scheme = match[1].toLowerCase();
  const params = {};
  for (const part of match[2].matchAll(/([A-Za-z_][A-Za-z0-9_-]*)="([^"]*)"/g)) {
    params[part[1].toLowerCase()] = part[2];
  }
  return { scheme, params, raw: header };
}

async function fetchWithTimeout(url, options = {}, timeoutMs = 30000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...options, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

async function acquireToken(challenge, username, password) {
  const params = challenge.params || {};
  if (!params.realm) throw new DeployError('镜像仓库返回 Bearer 认证但缺少 realm');
  const url = new URL(params.realm);
  if (params.service) url.searchParams.set('service', params.service);
  if (params.scope) url.searchParams.set('scope', params.scope);
  const headers = {};
  if (username) headers.Authorization = basicAuth(username, password || '');
  const response = await fetchWithTimeout(url.toString(), { headers }, 30000);
  if (!response.ok) {
    throw new DeployError(`获取仓库令牌失败: HTTP ${response.status}`, {
      code: 'REGISTRY_AUTH_FAILED',
      details: { realm: params.realm }
    });
  }
  const payload = await response.json();
  const token = payload.token || payload.access_token;
  if (!token) throw new DeployError('仓库令牌响应中缺少 token');
  return token;
}

export async function registryFetch(registryConfig, pathname, options = {}) {
  const registry = normalizeRegistry(registryConfig.url || registryConfig.registry);
  const url = `${baseUrl(registry, registryConfig.insecure)}${pathname}`;
  const headers = { ...(options.headers || {}) };
  const username = registryConfig.username || '';
  const password = registryConfig.password || '';
  if (options.basic && username) headers.Authorization = basicAuth(username, password);

  let response;
  try {
    response = await fetchWithTimeout(url, { method: options.method || 'GET', headers }, options.timeoutMs || 30000);
  } catch (error) {
    throw new DeployError(`访问镜像仓库失败: ${error.cause?.message || error.message}`, {
      code: 'REGISTRY_UNREACHABLE',
      details: { url }
    });
  }
  if (response.status === 401) {
    const challenge = parseWwwAuthenticate(response.headers.get('www-authenticate'));
    if (!challenge) {
      throw new DeployError('镜像仓库要求认证，但未返回可识别的 WWW-Authenticate 头', {
        code: 'REGISTRY_AUTH_FAILED'
      });
    }
    if (challenge.scheme === 'bearer') {
      const token = await acquireToken(challenge, username, password);
      headers.Authorization = `Bearer ${token}`;
    } else if (challenge.scheme === 'basic' && username) {
      headers.Authorization = basicAuth(username, password);
    } else {
      throw new DeployError('镜像仓库要求认证，请提供用户名和密码', { code: 'REGISTRY_AUTH_REQUIRED' });
    }
    try {
      response = await fetchWithTimeout(url, { method: options.method || 'GET', headers }, options.timeoutMs || 30000);
    } catch (error) {
      throw new DeployError(`访问镜像仓库失败: ${error.cause?.message || error.message}`, {
        code: 'REGISTRY_UNREACHABLE',
        details: { url }
      });
    }
  }
  return { response, url, registry };
}

export async function registryFromArgs(args = {}) {
  const { config } = await loadConfig();
  let registryConfig = null;
  if (args.registryName && config.registries[args.registryName]) {
    registryConfig = { ...config.registries[args.registryName] };
  } else if (args.registry) {
    const key = String(args.registry).trim();
    if (config.registries[key]) {
      registryConfig = { ...config.registries[key] };
    } else {
      registryConfig = { name: key, url: key, username: '', passwordEnv: '', insecure: false };
    }
  } else {
    const defaults = Object.values(config.registries).filter((item) => item.isDefault);
    if (defaults.length === 1) registryConfig = { ...defaults[0] };
  }
  if (!registryConfig) {
    throw new DeployError('未指定镜像仓库，请传入 registry 或先在仓库配置中设置默认仓库', {
      code: 'REGISTRY_NOT_CONFIGURED'
    });
  }
  if (args.username !== undefined) registryConfig.username = args.username;
  if (args.password !== undefined) registryConfig.password = args.password;
  if (args.passwordEnv !== undefined) registryConfig.passwordEnv = args.passwordEnv;
  if (args.insecure !== undefined) registryConfig.insecure = Boolean(args.insecure);
  if (!registryConfig.url) registryConfig.url = registryConfig.registry;
  registryConfig.url = normalizeRegistry(registryConfig.url);
  if (!registryConfig.password && registryConfig.passwordEnv) {
    registryConfig.password = process.env[registryConfig.passwordEnv] || '';
  }
  if (!registryConfig.password && process.env.DEPLOY_REGISTRY_PASSWORD) {
    registryConfig.password = process.env.DEPLOY_REGISTRY_PASSWORD;
  }
  return registryConfig;
}

export async function probeRegistry(registryConfig) {
  const { response, url, registry } = await registryFetch(registryConfig, '/v2/', { method: 'GET' });
  return {
    registry,
    url,
    status: response.status,
    ok: response.ok,
    authenticated: Boolean(registryConfig.username),
    message: response.ok ? '仓库连接正常' : `仓库返回 HTTP ${response.status}`
  };
}

export async function listTags(registryConfig, repository, options = {}) {
  const repo = String(repository || '').replace(/^\/+|\/+$/g, '');
  if (!repo) throw new DeployError('缺少 repository');
  const limit = Math.max(1, Math.min(1000, Number(options.limit || 100)));
  const { response, registry } = await registryFetch(registryConfig, `/v2/${repo}/tags/list?n=${limit}`, {
    headers: { Accept: 'application/json' }
  });
  if (!response.ok) throw new DeployError(`读取仓库 tag 失败: HTTP ${response.status}`);
  const payload = await response.json();
  return { registry, repository: repo, tags: Array.isArray(payload.tags) ? payload.tags : [] };
}

export async function resolveDigest(registryConfig, repository, tag = 'latest') {
  const repo = String(repository || '').replace(/^\/+|\/+$/g, '');
  const reference = String(tag || 'latest').trim();
  if (!repo || !reference) throw new DeployError('缺少 repository 或 tag');
  const pathname = `/v2/${repo}/manifests/${encodeURIComponent(reference)}`;
  let { response, registry, url } = await registryFetch(registryConfig, pathname, {
    method: 'HEAD',
    headers: { Accept: MANIFEST_ACCEPT }
  });
  let digest = response.headers.get('docker-content-digest');
  if (!digest) {
    ({ response, registry, url } = await registryFetch(registryConfig, pathname, {
      method: 'GET',
      headers: { Accept: MANIFEST_ACCEPT }
    }));
    if (!response.ok) throw new DeployError(`读取镜像 manifest 失败: HTTP ${response.status}`, { details: { url } });
    const body = Buffer.from(await response.arrayBuffer());
    digest = `sha256:${crypto.createHash('sha256').update(body).digest('hex')}`;
  }
  if (!response.ok) throw new DeployError(`解析镜像 digest 失败: HTTP ${response.status}`, { details: { url } });
  return { registry, repository: repo, tag: reference, digest, url };
}

function credentialIsPresent(registryConfig) {
  if (!registryConfig.username) return false;
  if (!registryConfig.password) {
    throw new DeployError('已配置仓库用户名，但未提供密码', {
      code: 'REGISTRY_PASSWORD_MISSING',
      hint: `请传入 password，或设置环境变量 ${registryConfig.passwordEnv || 'DEPLOY_REGISTRY_PASSWORD'}。`
    });
  }
  return true;
}

export async function dockerLoginLocal(registryConfig) {
  if (!credentialIsPresent(registryConfig)) return { skipped: true, message: '未配置仓库用户名，跳过 docker login' };
  const docker = requireBinary('docker');
  const result = await run(docker, ['login', registryConfig.url, '-u', registryConfig.username, '--password-stdin'], {
    input: registryConfig.password,
    timeoutMs: 60000
  });
  if (result.code !== 0) {
    throw new DeployError(`镜像仓库登录失败: ${result.stderr.trim() || result.stdout.trim()}`, {
      code: 'REGISTRY_LOGIN_FAILED'
    });
  }
  return { success: true, message: `已登录 ${registryConfig.url}` };
}

export async function dockerLogoutLocal(registryConfig) {
  const docker = requireBinary('docker');
  const result = await run(docker, ['logout', registryConfig.url], { timeoutMs: 30000 });
  return { success: result.code === 0, message: result.code === 0 ? `已退出 ${registryConfig.url}` : result.stderr.trim() };
}

export async function dockerLoginRemote(hostConfig, registryConfig) {
  if (!credentialIsPresent(registryConfig)) return { skipped: true, message: '未配置仓库用户名，跳过 docker login' };
  const encoded = Buffer.from(registryConfig.password).toString('base64');
  const script = `set -e
printf '%s' ${shellQuote(encoded)} | base64 -d | docker login ${shellQuote(registryConfig.url)} -u ${shellQuote(registryConfig.username)} --password-stdin >/dev/null
`;
  const result = await runRemote(hostConfig, script, { timeoutMs: 60000 });
  return { success: result.code === 0, message: `远程已登录 ${registryConfig.url}` };
}

export async function dockerLogoutRemote(hostConfig, registryConfig) {
  const result = await runRemote(hostConfig, `docker logout ${shellQuote(registryConfig.url)} || true\n`, { timeoutMs: 30000 });
  return { success: result.code === 0, message: `远程已退出 ${registryConfig.url}` };
}
