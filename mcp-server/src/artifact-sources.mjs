import fs from 'node:fs/promises';
import path from 'node:path';
import { DeployError } from './errors.mjs';
import { appConfigDir } from './projects.mjs';
import { isObject } from './util.mjs';

export const ARTIFACT_SOURCES_FILE = 'artifact-sources.json';
export const ARTIFACT_TOKENS_FILE = 'artifact-tokens.json';
export const ARTIFACT_RELEASE_CACHE_FILE = 'artifact-releases-cache.json';
export const ARTIFACT_BUNDLES_DIR = 'bundles';

let sourceQueue = Promise.resolve();
let tokenQueue = Promise.resolve();
let cacheQueue = Promise.resolve();

export function artifactSourcePath() {
  return path.join(appConfigDir(), ARTIFACT_SOURCES_FILE);
}

export function artifactTokenPath() {
  return path.join(appConfigDir(), ARTIFACT_TOKENS_FILE);
}

export function artifactReleaseCachePath() {
  return path.join(appConfigDir(), ARTIFACT_RELEASE_CACHE_FILE);
}

export function artifactCacheRoot() {
  return path.join(appConfigDir(), ARTIFACT_BUNDLES_DIR);
}

async function readJson(file, label) {
  try {
    return JSON.parse(await fs.readFile(file, 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw new DeployError(`读取${label}失败：${file}`, {
      code: 'ARTIFACT_CONFIG_READ_FAILED',
      details: { path: file, error: error.message }
    });
  }
}

async function writePrivateJson(file, value) {
  const dir = path.dirname(file);
  await fs.mkdir(dir, { recursive: true, mode: 0o700 });
  if (process.platform !== 'win32') await fs.chmod(dir, 0o700).catch(() => {});
  const temp = `${file}.${process.pid}.${Date.now()}.tmp`;
  await fs.writeFile(temp, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  if (process.platform !== 'win32') await fs.chmod(temp, 0o600).catch(() => {});
  await fs.rename(temp, file);
  if (process.platform !== 'win32') await fs.chmod(file, 0o600).catch(() => {});
}

function normalizeSource(source) {
  return {
    id: String(source.id || ''),
    name: String(source.name || ''),
    type: String(source.type || 'yunxiaoFlow'),
    orgId: String(source.orgId || ''),
    pipelineId: String(source.pipelineId || ''),
    artifactName: String(source.artifactName || 'deploy-bundle'),
    version: String(source.version || ''),
    downloadUrl: String(source.downloadUrl || ''),
    hasToken: Boolean(source.hasToken),
    lastVersion: String(source.lastVersion || ''),
    lastBundleDir: String(source.lastBundleDir || '')
  };
}

export async function loadArtifactSources() {
  const file = artifactSourcePath();
  const raw = await readJson(file, '制品源配置');
  if (raw === null) return [];
  if (!isObject(raw) || !Array.isArray(raw.sources)) {
    throw new DeployError('制品源配置格式不正确，请检查本机制品源配置', {
      code: 'ARTIFACT_SOURCES_INVALID',
      details: { path: file }
    });
  }
  return raw.sources.filter(isObject).map(normalizeSource);
}

export async function saveArtifactSources(sources) {
  const file = artifactSourcePath();
  await writePrivateJson(file, { sources: sources.map(normalizeSource) });
  return file;
}

export function mutateArtifactSources(mutator) {
  const run = sourceQueue.then(async () => {
    const sources = await loadArtifactSources();
    const next = structuredClone(sources);
    const result = await mutator(next);
    const file = await saveArtifactSources(next);
    return { sources: next, path: file, result };
  });
  sourceQueue = run.catch(() => {});
  return run;
}

export function validateArtifactVersion(raw, label = '制品版本号') {
  const version = String(raw ?? '').trim();
  if (!version || version === 'manual') {
    throw new DeployError(`${label}不能为空`, { code: 'ARTIFACT_VERSION_INVALID' });
  }
  if (version.length > 128 || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(version)) {
    throw new DeployError(`${label}格式无效，请使用字母、数字、点、短横线或下划线，以字母或数字开头，最多 128 个字符`, {
      code: 'ARTIFACT_VERSION_INVALID'
    });
  }
  return version;
}

export function safeVersionKey(version) {
  return String(version || '')
    .trim()
    .replace(/[^A-Za-z0-9._-]/g, '_');
}

export function validateDownloadAddress(raw) {
  const value = String(raw ?? '').trim();
  if (!value) throw new DeployError('请填写制品下载地址', { code: 'ARTIFACT_URL_REQUIRED' });
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new DeployError('制品下载地址格式无效', { code: 'ARTIFACT_URL_INVALID' });
  }
  if (parsed.protocol !== 'https:') {
    throw new DeployError('制品下载地址必须使用 HTTPS', { code: 'ARTIFACT_URL_INSECURE' });
  }
  return value;
}

export function validateArtifactSource(input, existing = null) {
  if (!isObject(input)) throw new DeployError('制品源配置必须是对象', { code: 'ARTIFACT_SOURCE_INVALID' });
  const name = String(input.name ?? '').trim();
  if (!name) throw new DeployError('请填写制品源名称', { code: 'ARTIFACT_SOURCE_NAME_REQUIRED' });
  if ([...name].length > 60 || /[\r\n\0]/.test(name)) {
    throw new DeployError('制品源名称格式无效，最多 60 个字符', { code: 'ARTIFACT_SOURCE_NAME_INVALID' });
  }

  const type = String(input.type ?? existing?.type ?? 'yunxiaoFlow').trim() || 'yunxiaoFlow';
  if (!['yunxiaoFlow', 'url'].includes(type)) {
    throw new DeployError(`不支持的制品源类型：${type}`, { code: 'ARTIFACT_SOURCE_TYPE_INVALID' });
  }

  const orgId = String(input.orgId ?? '').trim();
  const pipelineId = String(input.pipelineId ?? '').trim();
  if (type === 'yunxiaoFlow') {
    if (!orgId || !pipelineId) {
      throw new DeployError('云效流水线制品源需要填写组织 ID 与流水线 ID', { code: 'ARTIFACT_FLOW_REQUIRED' });
    }
    if (/[\s\r\n\0]/.test(orgId) || /[\s\r\n\0]/.test(pipelineId)) {
      throw new DeployError('组织 ID 与流水线 ID 不能包含空白字符', { code: 'ARTIFACT_FLOW_INVALID' });
    }
  }

  let version = String(input.version ?? existing?.version ?? '').trim();
  let downloadUrl = String(input.downloadUrl ?? existing?.downloadUrl ?? '').trim();
  if (type === 'url') {
    version = validateArtifactVersion(version || existing?.lastVersion || '');
    downloadUrl = validateDownloadAddress(downloadUrl);
  } else {
    version = '';
    if (downloadUrl) downloadUrl = validateDownloadAddress(downloadUrl);
  }

  const artifactName = String(input.artifactName ?? existing?.artifactName ?? 'deploy-bundle').trim() || 'deploy-bundle';
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(artifactName)) {
    throw new DeployError('制品名格式无效，请使用字母、数字、点、短横线或下划线', { code: 'ARTIFACT_NAME_INVALID' });
  }

  return {
    id: String(existing?.id || input.id || '').trim(),
    name,
    type,
    orgId,
    pipelineId,
    artifactName,
    version,
    downloadUrl,
    hasToken: Boolean(existing?.hasToken),
    lastVersion: String(existing?.lastVersion || ''),
    lastBundleDir: String(existing?.lastBundleDir || '')
  };
}

export function nextArtifactSourceId(sources) {
  const stamp = Date.now().toString(16);
  let candidate = `src-${stamp}`;
  let index = 1;
  while (sources.some((source) => source.id === candidate)) {
    candidate = `src-${stamp}-${index}`;
    index += 1;
  }
  return candidate;
}

export function findArtifactSource(sources, selector) {
  const value = String(selector || '').trim();
  if (!value) return null;
  const byId = sources.filter((source) => source.id === value);
  if (byId.length === 1) return byId[0];
  const byName = sources.filter((source) => source.name === value);
  if (byName.length === 1) return byName[0];
  if (byName.length > 1) {
    throw new DeployError(`制品源「${value}」名称重复，请改用制品源 ID`, { code: 'ARTIFACT_SOURCE_AMBIGUOUS' });
  }
  return null;
}

export function requireArtifactSource(sources, selector) {
  const source = findArtifactSource(sources, selector);
  if (!source) {
    throw new DeployError(`制品源不存在：${selector || ''}`, {
      code: 'ARTIFACT_SOURCE_NOT_FOUND',
      hint: '请先查看制品源列表，确认名称或 ID。'
    });
  }
  return source;
}

async function loadTokenFile() {
  const raw = await readJson(artifactTokenPath(), '制品源令牌');
  if (raw === null) return {};
  if (!isObject(raw)) {
    throw new DeployError('制品源令牌文件格式不正确', { code: 'ARTIFACT_TOKEN_INVALID' });
  }
  return raw;
}

export async function readArtifactToken(sourceId) {
  const tokens = await loadTokenFile();
  const value = tokens[String(sourceId || '').trim()];
  return typeof value === 'string' && value.trim() ? value : '';
}

export function mutateArtifactTokens(mutator) {
  const run = tokenQueue.then(async () => {
    const tokens = await loadTokenFile();
    const result = await mutator(tokens);
    await writePrivateJson(artifactTokenPath(), tokens);
    return result;
  });
  tokenQueue = run.catch(() => {});
  return run;
}

export async function writeArtifactToken(sourceId, token) {
  const value = String(token || '').trim();
  if (!value) throw new DeployError('令牌不能为空', { code: 'ARTIFACT_TOKEN_EMPTY' });
  await mutateArtifactTokens((tokens) => {
    tokens[sourceId] = value;
    delete tokens[`__keychain_migrated:${sourceId}`];
  });
}

export async function removeArtifactToken(sourceId) {
  await mutateArtifactTokens((tokens) => {
    delete tokens[sourceId];
    delete tokens[`__keychain_migrated:${sourceId}`];
  });
}

export function sourceSignature(source) {
  return [source.type, source.orgId, source.pipelineId, source.artifactName, source.downloadUrl].join('|');
}

async function loadReleaseCacheFile() {
  const raw = await readJson(artifactReleaseCachePath(), '制品版本缓存');
  if (raw === null) return { sources: {} };
  if (!isObject(raw) || !isObject(raw.sources)) {
    return { sources: {} };
  }
  return { sources: raw.sources };
}

export async function loadReleaseCache(source) {
  const file = await loadReleaseCacheFile();
  const entry = file.sources[source.id];
  if (!isObject(entry) || entry.signature !== sourceSignature(source)) {
    return { items: [], hasMore: false, cachedAt: 0 };
  }
  return {
    items: Array.isArray(entry.items) ? entry.items : [],
    hasMore: Boolean(entry.hasMore),
    cachedAt: Number(entry.cachedAt) || 0
  };
}

export function mutateReleaseCache(mutator) {
  const run = cacheQueue.then(async () => {
    const file = await loadReleaseCacheFile();
    const result = await mutator(file);
    await writePrivateJson(artifactReleaseCachePath(), file);
    return result;
  });
  cacheQueue = run.catch(() => {});
  return run;
}

export async function storeReleasePage(source, pageNumber, items, hasMore) {
  const signature = sourceSignature(source);
  const sanitized = items.map((item) => ({ ...item, downloadUrl: '' }));
  await mutateReleaseCache((file) => {
    const entry = isObject(file.sources[source.id]) ? file.sources[source.id] : {};
    if (entry.signature !== signature) {
      entry.signature = signature;
      entry.items = [];
      entry.hasMore = false;
    }
    if (pageNumber <= 1) {
      entry.items = sanitized;
    } else {
      for (const item of sanitized) {
        if (!entry.items.some((known) => releaseCacheKey(known) === releaseCacheKey(item))) {
          entry.items.push(item);
        }
      }
    }
    entry.hasMore = Boolean(hasMore);
    entry.cachedAt = Date.now();
    file.sources[source.id] = entry;
  });
}

export async function removeReleaseCacheEntry(sourceId) {
  await mutateReleaseCache((file) => {
    delete file.sources[sourceId];
  });
}

export function releaseCacheKey(item) {
  return `${item.runId || 0}|${item.artifactName || ''}|${item.version || ''}`;
}

export async function readBundleMetadata(dir) {
  const manifest = path.join(dir, 'bundle.json');
  try {
    const raw = JSON.parse(await fs.readFile(manifest, 'utf8'));
    if (!isObject(raw) || !isObject(raw.metadata)) return null;
    return raw;
  } catch {
    return null;
  }
}

export function bundleMatchesVersion(meta, version) {
  return String(meta?.metadata?.version || '').trim() === String(version || '').trim();
}

export async function cachedBundleDir(source, version) {
  const expected = path.join(artifactCacheRoot(), source.id, safeVersionKey(version));
  const expectedMeta = await readBundleMetadata(expected);
  if (expectedMeta && bundleMatchesVersion(expectedMeta, version)) return expected;
  const last = String(source.lastBundleDir || '').trim();
  if (!last || String(source.lastVersion || '').trim() !== String(version || '').trim()) return '';
  const lastMeta = await readBundleMetadata(last);
  if (lastMeta && bundleMatchesVersion(lastMeta, version)) return last;
  return '';
}

export async function listCachedBundles(source) {
  const root = path.join(artifactCacheRoot(), source.id);
  const dirs = [];
  try {
    const entries = await fs.readdir(root, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.isDirectory()) dirs.push(path.join(root, entry.name));
    }
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  const last = String(source.lastBundleDir || '').trim();
  if (last && !dirs.includes(last)) dirs.push(last);
  const bundles = [];
  for (const dir of dirs) {
    const meta = await readBundleMetadata(dir);
    if (!meta) continue;
    const version = String(meta.metadata.version || '').trim();
    if (!version) continue;
    bundles.push({
      dir,
      version,
      product: String(meta.metadata.product || ''),
      commit: String(meta.metadata.commit || ''),
      commitShort: String(meta.metadata.commitShort || ''),
      branch: String(meta.metadata.branch || ''),
      builtAt: Date.parse(meta.metadata.builtAt) || 0,
      sizeBytes: (Array.isArray(meta.files) ? meta.files : []).reduce((total, item) => total + (Number(item.size) || 0), 0)
    });
  }
  return bundles.sort((left, right) => right.builtAt - left.builtAt || right.version.localeCompare(left.version));
}
