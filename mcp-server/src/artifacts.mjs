import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { Bundle } from './bundle.mjs';
import { appVersion } from './config.mjs';
import { DeployError } from './errors.mjs';
import {
  artifactCacheRoot,
  artifactSourcePath,
  bundleMatchesVersion,
  cachedBundleDir,
  listCachedBundles,
  loadArtifactSources,
  mutateArtifactSources,
  readArtifactToken,
  readBundleMetadata,
  releaseCacheKey,
  loadReleaseCache,
  safeVersionKey,
  sourceSignature,
  storeReleasePage,
  validateArtifactVersion
} from './artifact-sources.mjs';
import { isObject } from './util.mjs';

const execFileAsync = promisify(execFile);
const FLOW_API = 'https://openapi-rdc.aliyuncs.com/oapi/v1/flow/organizations';
const FLOW_WEB = 'https://flow.aliyun.com/pipelines';
const FLOW_PAGE_SIZE = 20;
const MAX_ARTIFACT_BYTES = 5 * 1024 * 1024 * 1024;
const MAX_EXTRACTED_BYTES = MAX_ARTIFACT_BYTES * 4;
const MAX_EXTRACTED_FILES = 200_000;
const SEARCH_PAGE_LIMIT = 20;

const installLocks = new Map();

function fail(message, code, hint, details) {
  return new DeployError(message, { code, hint, details });
}

function jsonText(value, key) {
  return isObject(value) && typeof value[key] === 'string' ? value[key] : '';
}

function jsonNumber(value, key) {
  if (!isObject(value)) return 0;
  const number = Number(value[key]);
  return Number.isFinite(number) && number > 0 ? number : 0;
}

function parseEmbedded(value) {
  if (typeof value !== 'string') return null;
  const text = value.trim();
  if (!(text.startsWith('{') || text.startsWith('['))) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function collectArtifacts(value, output = []) {
  if (Array.isArray(value)) {
    for (const item of value) collectArtifacts(item, output);
    return output;
  }
  if (isObject(value)) {
    if (Object.hasOwn(value, 'downloadUrl') && Object.hasOwn(value, 'md5')) output.push(value);
    for (const item of Object.values(value)) collectArtifacts(item, output);
    return output;
  }
  const embedded = parseEmbedded(value);
  if (embedded) collectArtifacts(embedded, output);
  return output;
}

function findString(value, key) {
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findString(item, key);
      if (found) return found;
    }
    return '';
  }
  if (isObject(value)) {
    if (typeof value[key] === 'string' && value[key].trim()) return value[key];
    for (const [name, item] of Object.entries(value)) {
      if ((name === key || name.endsWith(`.${key}`)) && typeof item === 'string' && item.trim()) return item;
    }
    for (const item of Object.values(value)) {
      const found = findString(item, key);
      if (found) return found;
    }
    return '';
  }
  const embedded = parseEmbedded(value);
  return embedded ? findString(embedded, key) : '';
}

function findNumber(value, key) {
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findNumber(item, key);
      if (found) return found;
    }
    return 0;
  }
  if (isObject(value)) {
    const own = Number(value[key]);
    if (Number.isFinite(own) && own > 0) return own;
    for (const [name, item] of Object.entries(value)) {
      if (name === key || name.endsWith(`.${key}`)) {
        const number = Number(item);
        if (Number.isFinite(number) && number > 0) return number;
      }
    }
    for (const item of Object.values(value)) {
      const found = findNumber(item, key);
      if (found) return found;
    }
    return 0;
  }
  const embedded = parseEmbedded(value);
  return embedded ? findNumber(embedded, key) : 0;
}

function pickArtifact(value, artifactName) {
  const candidates = collectArtifacts(value);
  return candidates.find((item) => {
    const name = jsonText(item, 'artifact');
    return name === artifactName || name === `${artifactName}.tgz` || name === `${artifactName}.tar.gz`;
  }) || null;
}

function releaseFromFlowDetail(source, runId, detail) {
  const artifact = pickArtifact(detail, source.artifactName);
  if (!artifact) return null;
  const downloadUrl = jsonText(artifact, 'downloadUrl').trim();
  if (!downloadUrl) return null;
  const rawVersion = artifact.version ?? '';
  let version;
  try {
    version = validateArtifactVersion(rawVersion);
  } catch {
    throw fail(`运行 #${runId} 的制品版本号无效，请重新构建并上传带版本号的制品`, 'ARTIFACT_VERSION_INVALID');
  }
  return {
    runId,
    runUrl: `${FLOW_WEB}/${encodeURIComponent(source.pipelineId)}/runs/${runId}`,
    runStatus: jsonText(detail, 'status'),
    version,
    commit: findString(detail, 'CI_COMMIT_SHA'),
    commitShort: findString(detail, 'CI_COMMIT_ID'),
    commitTitle: findString(detail, 'CI_COMMIT_TITLE'),
    branch: findString(detail, 'CI_COMMIT_REF_NAME'),
    builtAt: findNumber(detail, 'startTime'),
    artifactName: jsonText(artifact, 'artifact'),
    repoId: jsonText(artifact, 'repoId'),
    sizeBytes: Math.max(jsonNumber(artifact, 'fileSize'), jsonNumber(artifact, 'size'), findNumber(detail, 'fileSize')),
    md5: jsonText(artifact, 'md5'),
    downloadUrl,
    localDir: ''
  };
}

function releaseFromBundleMetadata(source, dir, meta) {
  const version = String(meta?.metadata?.version || '').trim();
  if (!version) return null;
  return {
    runId: 0,
    runUrl: '',
    runStatus: 'cached',
    version,
    commit: String(meta.metadata.commit || ''),
    commitShort: String(meta.metadata.commitShort || ''),
    commitTitle: '',
    branch: String(meta.metadata.branch || ''),
    builtAt: Date.parse(meta.metadata.builtAt) || 0,
    artifactName: source.artifactName,
    repoId: '',
    sizeBytes: (Array.isArray(meta.files) ? meta.files : []).reduce((sum, item) => sum + (Number(item.size) || 0), 0),
    md5: '',
    downloadUrl: '',
    localDir: dir
  };
}

async function attachLocalBundle(source, release) {
  const dir = await cachedBundleDir(source, release.version);
  if (dir) release.localDir = dir;
  return release;
}

async function urlRelease(source) {
  const version = validateArtifactVersion(source.version || source.lastVersion, '制品版本号');
  const release = {
    runId: 0,
    runUrl: '',
    runStatus: 'manual',
    version,
    commit: '',
    commitShort: '',
    commitTitle: '',
    branch: '',
    builtAt: 0,
    artifactName: source.artifactName,
    repoId: '',
    sizeBytes: 0,
    md5: '',
    downloadUrl: source.downloadUrl,
    localDir: ''
  };
  return attachLocalBundle(source, release);
}

async function flowToken(source, token) {
  const value = String(token || '').trim() || await readArtifactToken(source);
  if (!value) {
    throw fail('尚未配置云效个人访问令牌', 'ARTIFACT_TOKEN_REQUIRED', '请编辑制品源填写令牌，或在本次调用中传入 token。');
  }
  return value;
}

async function readJsonResponse(response, label) {
  const body = await response.text();
  if (response.status === 401 || response.status === 403) {
    throw fail(`${label}令牌无效或权限不足，请重新填写个人访问令牌`, 'ARTIFACT_TOKEN_INVALID');
  }
  if (!response.ok) throw fail(`${label}返回 HTTP ${response.status}`, 'ARTIFACT_HTTP_ERROR');
  try {
    return JSON.parse(body);
  } catch {
    throw fail(`${label}响应不是有效 JSON`, 'ARTIFACT_RESPONSE_INVALID');
  }
}

async function flowRunItems(source, token, page, fetchImpl) {
  const url = `${FLOW_API}/${encodeURIComponent(source.orgId)}/pipelines/${encodeURIComponent(source.pipelineId)}/runs?page=${page}&perPage=${FLOW_PAGE_SIZE}`;
  let response;
  try {
    response = await fetchImpl(url, {
      headers: { 'x-yunxiao-token': token, Accept: 'application/json' },
      signal: AbortSignal.timeout(30_000)
    });
  } catch (error) {
    throw fail(error.name === 'TimeoutError' ? '读取云效运行列表超时' : '无法连接云效，请检查网络后重试', 'ARTIFACT_NETWORK_ERROR');
  }
  const raw = await readJsonResponse(response, '云效接口');
  const items = Array.isArray(raw) ? raw : (Array.isArray(raw?.items) ? raw.items : (Array.isArray(raw?.result) ? raw.result : null));
  if (!items) throw fail('云效运行列表格式异常', 'ARTIFACT_RESPONSE_INVALID');
  return items;
}

async function flowRunDetail(source, token, runId, fetchImpl) {
  const url = `${FLOW_API}/${encodeURIComponent(source.orgId)}/pipelines/${encodeURIComponent(source.pipelineId)}/runs/${runId}`;
  let response;
  try {
    response = await fetchImpl(url, {
      headers: { 'x-yunxiao-token': token, Accept: 'application/json' },
      signal: AbortSignal.timeout(30_000)
    });
  } catch (error) {
    throw fail(error.name === 'TimeoutError' ? '读取云效构建详情超时' : '无法连接云效，请检查网络后重试', 'ARTIFACT_NETWORK_ERROR');
  }
  return readJsonResponse(response, '云效接口');
}

async function loadFlowPage(source, page, { token, fetchImpl, persist = true }) {
  const accessToken = await flowToken(source, token);
  const runs = await flowRunItems(source, accessToken, page, fetchImpl);
  const releases = [];
  for (const run of runs) {
    if (jsonText(run, 'status') !== 'SUCCESS') continue;
    const runId = jsonNumber(run, 'pipelineRunId');
    if (!runId) continue;
    const detail = await flowRunDetail(source, accessToken, runId, fetchImpl);
    const release = releaseFromFlowDetail(source, runId, detail);
    if (release) releases.push(await attachLocalBundle(source, release));
  }
  releases.sort((left, right) => right.builtAt - left.builtAt || right.runId - left.runId);
  const result = {
    items: releases,
    page,
    perPage: FLOW_PAGE_SIZE,
    hasMore: runs.length >= FLOW_PAGE_SIZE
  };
  if (persist) await storeReleasePage(source, page, releases, result.hasMore);
  return result;
}

async function mergeCachedBundles(source, items) {
  for (const bundle of await listCachedBundles(source)) {
    const index = items.findIndex((item) => item.version === bundle.version);
    if (index >= 0) {
      if (!items[index].localDir) items[index].localDir = bundle.dir;
      continue;
    }
    const dirMeta = await readBundleMetadata(bundle.dir);
    const release = releaseFromBundleMetadata(source, bundle.dir, dirMeta);
    if (release) items.push(release);
  }
  items.sort((left, right) => right.builtAt - left.builtAt || right.runId - left.runId);
  return items;
}

export async function listArtifactReleases(source, options = {}) {
  const page = Math.max(1, Math.floor(Number(options.page) || 1));
  const fetchImpl = options.fetchImpl || globalThis.fetch;
  if (source.type === 'url') {
    const release = await urlRelease(source);
    return { items: [release], page: 1, perPage: 1, hasMore: false };
  }
  if (options.cachedOnly) {
    const cached = await loadReleaseCache(source);
    const items = await mergeCachedBundles(source, cached.items.map((item) => ({ ...item, downloadUrl: '' })));
    return { items, page: 1, perPage: FLOW_PAGE_SIZE, hasMore: cached.hasMore, cachedAt: cached.cachedAt };
  }
  if (typeof fetchImpl !== 'function') throw fail('当前运行环境不支持网络请求', 'ARTIFACT_FETCH_UNAVAILABLE');
  return loadFlowPage(source, page, { token: options.token, fetchImpl, persist: options.persist !== false });
}

function validateDownloadUrl(url, source) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    throw fail('下载地址格式无效', 'ARTIFACT_URL_INVALID');
  }
  if (parsed.protocol !== 'https:') throw fail('仅允许通过 HTTPS 下载交付包', 'ARTIFACT_URL_INSECURE');
  if (source.type === 'url') {
    if (parsed.toString() !== new URL(source.downloadUrl).toString()) {
      throw fail('下载地址与制品源配置不一致，已拒绝请求', 'ARTIFACT_URL_MISMATCH');
    }
    return;
  }
  const host = parsed.hostname.toLowerCase();
  const allowed = host === 'aliyuncs.com' || host.endsWith('.aliyuncs.com') || host === 'aliyun.com' || host.endsWith('.aliyun.com');
  if (!allowed) throw fail(`构建物下载域名不在允许范围内：${host}`, 'ARTIFACT_URL_HOST_DENIED');
}

export async function resolveArtifactRelease(source, options = {}) {
  const fetchImpl = options.fetchImpl || globalThis.fetch;
  const runId = Number(options.runId) || 0;
  const version = String(options.version || '').trim();
  if (source.type === 'url') {
    const release = await urlRelease(source);
    if (version && release.version !== version) {
      throw fail(`制品源版本为 ${release.version}，与所选版本 ${version} 不一致`, 'ARTIFACT_VERSION_MISMATCH');
    }
    return release;
  }
  if (typeof fetchImpl !== 'function') throw fail('当前运行环境不支持网络请求', 'ARTIFACT_FETCH_UNAVAILABLE');
  const token = await flowToken(source, options.token);
  if (runId > 0) {
    const release = releaseFromFlowDetail(source, runId, await flowRunDetail(source, token, runId, fetchImpl));
    if (!release) throw fail('未在该运行中找到构建物上传结果', 'ARTIFACT_RELEASE_NOT_FOUND');
    if (version && release.version !== version) {
      throw fail(`运行 #${runId} 的制品版本为 ${release.version}，与所选版本 ${version} 不一致`, 'ARTIFACT_VERSION_MISMATCH');
    }
    return attachLocalBundle(source, release);
  }
  const pages = version ? SEARCH_PAGE_LIMIT : 1;
  for (let page = 1; page <= pages; page += 1) {
    const result = await loadFlowPage(source, page, { token, fetchImpl });
    const release = version
      ? result.items.find((item) => item.version === version)
      : result.items[0];
    if (release) return release;
    if (!result.hasMore) break;
  }
  throw fail(version ? `没有找到版本 ${version} 的制品` : '没有找到包含构建物的成功运行', 'ARTIFACT_RELEASE_NOT_FOUND');
}

function withInstallLock(key, handler) {
  const previous = installLocks.get(key) || Promise.resolve();
  const run = previous.then(handler, handler);
  installLocks.set(key, run);
  run.finally(() => {
    if (installLocks.get(key) === run) installLocks.delete(key);
  }).catch(() => {});
  return run;
}

async function downloadToFile(url, target, expectedTotal, onProgress) {
  let response;
  try {
    response = await globalThis.fetch(url, { redirect: 'follow', signal: AbortSignal.timeout(30 * 60 * 1000) });
  } catch (error) {
    throw fail(error.name === 'TimeoutError' ? '下载交付包超时，请检查网络后重试' : '下载交付包失败，请检查网络后重试', 'ARTIFACT_DOWNLOAD_FAILED');
  }
  if (!response.ok) throw fail(`下载交付包失败：HTTP ${response.status}`, 'ARTIFACT_DOWNLOAD_FAILED');
  const headerLength = Number(response.headers.get('content-length')) || 0;
  const total = headerLength || Number(expectedTotal) || 0;
  if (total > MAX_ARTIFACT_BYTES) throw fail(`交付包体积超过限制：${total} 字节`, 'ARTIFACT_TOO_LARGE');
  if (!response.body) throw fail('下载响应为空', 'ARTIFACT_DOWNLOAD_FAILED');

  const handle = await fs.open(target, 'wx', 0o600);
  const hash = crypto.createHash('md5');
  let downloaded = 0;
  try {
    const reader = response.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      const chunk = Buffer.from(value);
      downloaded += chunk.length;
      if (downloaded > MAX_ARTIFACT_BYTES) throw fail('交付包实际体积超过限制，已中止下载', 'ARTIFACT_TOO_LARGE');
      await handle.write(chunk);
      hash.update(chunk);
      onProgress?.(downloaded, total || downloaded);
    }
    await handle.sync();
  } catch (error) {
    await handle.close().catch(() => {});
    await fs.rm(target, { force: true }).catch(() => {});
    if (error instanceof DeployError) throw error;
    throw fail('读取下载数据失败，请检查网络后重试', 'ARTIFACT_DOWNLOAD_FAILED');
  }
  await handle.close();
  return { size: downloaded, md5: hash.digest('hex') };
}

function isSafeEntry(name) {
  if (!name || name.startsWith('/') || name.startsWith('\\')) return false;
  if (/^[A-Za-z]:/.test(name)) return false;
  return !name.split(/[\\/]/).some((part) => part === '..');
}

async function safeExtract(archive, destination) {
  let listing;
  try {
    listing = await execFileAsync('tar', ['-tzf', archive], { maxBuffer: 32 * 1024 * 1024 });
  } catch {
    throw fail('构建物不是有效的 tar.gz 压缩包', 'ARTIFACT_ARCHIVE_INVALID');
  }
  for (const line of String(listing.stdout || '').split('\n')) {
    const name = line.trim();
    if (name && !isSafeEntry(name)) throw fail(`构建物包含非法路径：${name}`, 'ARTIFACT_ARCHIVE_UNSAFE');
  }
  let verbose;
  try {
    verbose = await execFileAsync('tar', ['-tvzf', archive], { maxBuffer: 32 * 1024 * 1024 });
  } catch {
    throw fail('无法检查压缩包内容，请重新选择完整的交付包', 'ARTIFACT_ARCHIVE_INVALID');
  }
  for (const line of String(verbose.stdout || '').split('\n')) {
    if (!line) continue;
    if (!line.startsWith('-') && !line.startsWith('d')) {
      throw fail('交付包包含链接或特殊文件，无法解压，请重新打包后重试', 'ARTIFACT_ARCHIVE_UNSAFE');
    }
  }
  await fs.mkdir(destination, { recursive: true, mode: 0o700 });
  try {
    await execFileAsync('tar', ['-xzf', archive, '-C', destination], { maxBuffer: 32 * 1024 * 1024 });
  } catch {
    throw fail('解包构建物失败', 'ARTIFACT_EXTRACT_FAILED');
  }
}

async function countPayloadFiles(root, relative = '', totals = { files: 0, bytes: 0 }) {
  const entries = await fs.readdir(path.join(root, relative), { withFileTypes: true });
  for (const entry of entries) {
    const next = relative ? `${relative}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      await countPayloadFiles(root, next, totals);
      continue;
    }
    if (entry.name === '.DS_Store') continue;
    if (!entry.isFile()) throw fail('交付包包含链接或特殊文件，请重新打包后重试', 'ARTIFACT_ARCHIVE_UNSAFE');
    if (next === 'bundle.json') continue;
    const stat = await fs.stat(path.join(root, next));
    totals.files += 1;
    totals.bytes += stat.size;
    if (totals.files > MAX_EXTRACTED_FILES || totals.bytes > MAX_EXTRACTED_BYTES) {
      throw fail('交付包解压后体积或文件数量超过限制', 'ARTIFACT_TOO_LARGE');
    }
  }
  return totals;
}

async function installArtifactArchive(archive, destination, version) {
  const parent = path.dirname(destination);
  await fs.mkdir(parent, { recursive: true, mode: 0o700 });
  const token = `${process.pid}-${Date.now()}-${crypto.randomBytes(4).toString('hex')}`;
  const staging = path.join(parent, `.unpack-${token}`);
  const backup = path.join(parent, `.previous-${token}`);
  try {
    await safeExtract(archive, staging);
    const bundle = await Bundle.load(staging, { appVersion: await appVersion() });
    await bundle.verifyPayload();
    if (bundle.version() !== version) {
      throw fail(`交付包版本不匹配：选择的版本为 ${version}，包内版本为 ${bundle.version()}`, 'ARTIFACT_VERSION_MISMATCH');
    }
    const totals = await countPayloadFiles(staging);
    const existed = await fs.stat(destination).then(() => true).catch(() => false);
    if (existed) await fs.rename(destination, backup);
    try {
      await fs.rename(staging, destination);
    } catch (error) {
      if (existed) await fs.rename(backup, destination).catch(() => {});
      throw fail(`保存交付包失败：${error.message}`, 'ARTIFACT_INSTALL_FAILED');
    }
    if (existed) await fs.rm(backup, { recursive: true, force: true });
    return {
      dir: destination,
      version: bundle.version(),
      commit: bundle.descriptor.metadata.commitShort,
      files: totals.files,
      sizeBytes: totals.bytes
    };
  } finally {
    await fs.rm(staging, { recursive: true, force: true }).catch(() => {});
    await fs.rm(backup, { recursive: true, force: true }).catch(() => {});
  }
}

export async function downloadArtifact(source, release, options = {}) {
  if (!release?.downloadUrl) throw fail('下载地址为空', 'ARTIFACT_URL_REQUIRED');
  const url = source.type === 'url' ? source.downloadUrl : release.downloadUrl;
  validateDownloadUrl(url, source);
  const version = validateArtifactVersion(release.version, '制品版本号');
  if (source.type === 'url') {
    const expected = validateArtifactVersion(source.version || source.lastVersion, '制品版本号');
    if (expected !== version) throw fail('制品源版本已变更，请重新查看制品列表', 'ARTIFACT_VERSION_MISMATCH');
  }

  const destination = path.join(artifactCacheRoot(), source.id, safeVersionKey(version));
  return withInstallLock(`${source.id}:${safeVersionKey(version)}`, async () => {
    const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'deploy-artifact-'));
    const archive = path.join(tempDir, 'bundle.tgz');
    try {
      options.onProgress?.({ stage: 'download', downloaded: 0, total: release.sizeBytes || 0 });
      const result = await downloadToFile(url, archive, release.sizeBytes, (downloaded, total) => {
        options.onProgress?.({ stage: 'download', downloaded, total });
      });
      if (release.md5 && release.md5.trim() && result.md5.toLowerCase() !== release.md5.trim().toLowerCase()) {
        throw fail(`构建物校验失败：期望 MD5 ${release.md5}，实际 ${result.md5}`, 'ARTIFACT_DIGEST_MISMATCH');
      }
      options.onProgress?.({ stage: 'extract', downloaded: result.size, total: result.size });
      const installed = await installArtifactArchive(archive, destination, version);
      await mutateArtifactSources((sources) => {
        const target = sources.find((item) => item.id === source.id);
        if (target) {
          target.lastVersion = installed.version;
          target.lastBundleDir = installed.dir;
          target.hasToken = Boolean(source.hasToken);
        }
      });
      options.onProgress?.({ stage: 'done', downloaded: result.size, total: result.size });
      return {
        ...installed,
        sourceId: source.id,
        sourceName: source.name,
        md5: result.md5,
        archiveBytes: result.size
      };
    } finally {
      await fs.rm(tempDir, { recursive: true, force: true }).catch(() => {});
    }
  });
}

export async function artifactSourceOverview() {
  const sources = await loadArtifactSources();
  return Promise.all(sources.map(async (source) => ({
    ...source,
    localBundles: await listCachedBundles(source)
  })));
}

export async function artifactCachedPage(source) {
  if (source.type === 'url') return listArtifactReleases(source, { cachedOnly: true });
  const cached = await loadReleaseCache(source);
  const items = await mergeCachedBundles(source, cached.items.map((item) => ({ ...item, downloadUrl: '' })));
  return { items, page: 1, perPage: FLOW_PAGE_SIZE, hasMore: cached.hasMore, cachedAt: cached.cachedAt };
}

export const artifactPaths = Object.freeze({
  sources: artifactSourcePath,
  cacheRoot: artifactCacheRoot,
  signature: sourceSignature,
  cacheKey: releaseCacheKey,
  bundleMatchesVersion
});
