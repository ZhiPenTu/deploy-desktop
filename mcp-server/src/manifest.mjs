import fs from 'node:fs/promises';
import path from 'node:path';
import YAML from 'yaml';
import { DeployError } from './errors.mjs';
import { appVersion } from './profile.mjs';
import { expandHome, isObject } from './util.mjs';

const MANIFEST_API_VERSION = 'deploy.desktop/v1';
const MANIFEST_MAX_BYTES = 5 * 1024 * 1024;
const TOP_LEVEL_KEYS = ['apiVersion', 'kind', 'metadata', 'spec'];
const METADATA_KEYS = ['name', 'version', 'description', 'product', 'channel', 'createdAt', 'minToolVersion'];
const SPEC_KEYS = ['registry', 'namespace', 'platform', 'images', 'artifacts', 'checksums', 'signature'];
const IMAGE_KEYS = ['name', 'repository', 'tag', 'digest', 'sizeBytes', 'platform'];
const ARTIFACT_KEYS = ['name', 'path', 'url', 'sha256', 'sizeBytes'];

function assertKnownKeys(object, allowed, where) {
  if (!isObject(object)) throw new DeployError(`${where} 必须是对象`);
  const unknown = Object.keys(object).filter((key) => !allowed.includes(key));
  if (unknown.length > 0) {
    throw new DeployError(`${where} 包含未知字段: ${unknown.join(', ')}`, {
      hint: '未声明字段会被拒绝，避免拼写错误被静默忽略。'
    });
  }
}

export function normalizeRegistry(raw) {
  const trimmed = String(raw || '').trim();
  const withoutScheme = trimmed.replace(/^https?:\/\//i, '').replace(/\/+$/, '');
  if (!withoutScheme) throw new DeployError('镜像仓库地址不能为空');
  if (withoutScheme.includes('/') || withoutScheme.includes('@') || /[^A-Za-z0-9.:[\]-]/.test(withoutScheme)) {
    throw new DeployError(`镜像仓库地址无效: ${raw}`, {
      hint: '只填写 registry 主机名，例如 registry.cn-hangzhou.aliyuncs.com'
    });
  }
  return withoutScheme.toLowerCase();
}

function validateRepositoryPath(value, field) {
  const trimmed = String(value || '').trim();
  if (!trimmed || trimmed.startsWith('/') || trimmed.endsWith('/') || trimmed.includes('//') ||
      trimmed.includes('..') || trimmed.includes('@') || trimmed.includes(':') ||
      /[^A-Za-z0-9._/-]/.test(trimmed)) {
    throw new DeployError(`清单 ${field} 无效: ${value}`);
  }
  return trimmed;
}

function validatePlatform(value) {
  const parts = String(value || '').split('/');
  if (parts.length !== 2 || parts.some((part) => !part || !/^[A-Za-z0-9_.-]+$/.test(part))) {
    throw new DeployError(`平台格式无效: ${value}，应形如 linux/amd64`);
  }
  return value;
}

export function isValidDigest(value) {
  return /^sha256:[0-9a-f]{64}$/.test(String(value || ''));
}

export function validateManifest(manifest, options = {}) {
  assertKnownKeys(manifest, TOP_LEVEL_KEYS, '清单顶层');
  if (manifest.apiVersion !== MANIFEST_API_VERSION) {
    throw new DeployError(`不支持的清单版本: ${manifest.apiVersion}，当前仅支持 ${MANIFEST_API_VERSION}`);
  }
  if (manifest.kind !== 'ImageBundle') {
    throw new DeployError(`清单类型必须是 ImageBundle，当前为 ${manifest.kind}`);
  }
  assertKnownKeys(manifest.metadata, METADATA_KEYS, '清单 metadata');
  if (!String(manifest.metadata.name || '').trim()) throw new DeployError('清单 metadata.name 不能为空');
  if (!String(manifest.metadata.version || '').trim()) throw new DeployError('清单 metadata.version 不能为空');
  assertKnownKeys(manifest.spec, SPEC_KEYS, '清单 spec');
  if (!Array.isArray(manifest.spec.images) || manifest.spec.images.length === 0) {
    throw new DeployError('清单 spec.images 不能为空');
  }
  const registry = normalizeRegistry(options.registryOverride || manifest.spec.registry);
  const namespace = options.namespaceOverride !== undefined
    ? String(options.namespaceOverride || '').replace(/^\/+|\/+$/g, '')
    : (manifest.spec.namespace || '');
  if (namespace) validateRepositoryPath(namespace, 'namespace');
  const defaultPlatform = manifest.spec.platform;
  if (defaultPlatform) validatePlatform(defaultPlatform);

  const names = new Set();
  const images = [];
  for (const [index, image] of manifest.spec.images.entries()) {
    assertKnownKeys(image, IMAGE_KEYS, `清单 spec.images[${index}]`);
    const name = String(image.name || '').trim();
    if (!name) throw new DeployError(`第 ${index + 1} 个镜像缺少 name`);
    if (names.has(name)) throw new DeployError(`镜像名称重复: ${name}`);
    names.add(name);
    const repository = validateRepositoryPath(image.repository, 'repository');
    const digest = String(image.digest || '');
    if (!digest) throw new DeployError(`镜像 ${name} 缺少 digest，必须使用 sha256 摘要`);
    if (!isValidDigest(digest)) throw new DeployError(`镜像 ${name} 的 digest 无效，必须形如 sha256:<64位小写十六进制>`);
    if (image.tag !== undefined && image.tag !== null) {
      const tag = String(image.tag);
      if (!tag.trim() || tag.includes(' ') || tag.includes('@')) throw new DeployError(`镜像 ${name} 的 tag 无效`);
    }
    const platform = image.platform || defaultPlatform;
    if (platform) validatePlatform(platform);
    const repoPath = namespace ? `${namespace}/${repository}` : repository;
    images.push({
      name,
      repository: repoPath,
      reference: `${registry}/${repoPath}@${digest}`,
      digest,
      platform: platform || null,
      sizeBytes: Number(image.sizeBytes) || 0
    });
  }
  if (manifest.spec.artifacts !== undefined) {
    if (!Array.isArray(manifest.spec.artifacts)) throw new DeployError('清单 spec.artifacts 必须是数组');
    for (const [index, artifact] of manifest.spec.artifacts.entries()) {
      assertKnownKeys(artifact, ARTIFACT_KEYS, `清单 spec.artifacts[${index}]`);
      if (!String(artifact.name || '').trim()) throw new DeployError(`清单 spec.artifacts[${index}] 缺少 name`);
    }
  }
  return { manifest, registry, namespace, images };
}

async function readSource(source) {
  const trimmed = String(source || '').trim();
  if (!trimmed) throw new DeployError('请填写清单地址或本地文件路径');
  if (/^https?:\/\//i.test(trimmed)) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 30000);
    try {
      const response = await fetch(trimmed, {
        signal: controller.signal,
        headers: { 'User-Agent': 'deploy-desktop-mcp/0.1.2' }
      });
      if (!response.ok) throw new DeployError(`下载清单失败: HTTP ${response.status}`);
      const length = Number(response.headers.get('content-length') || 0);
      if (length > MANIFEST_MAX_BYTES) throw new DeployError('清单文件超过 5 MB 限制');
      const buffer = Buffer.from(await response.arrayBuffer());
      if (buffer.length > MANIFEST_MAX_BYTES) throw new DeployError('清单文件超过 5 MB 限制');
      return { content: buffer.toString('utf8'), source: trimmed };
    } catch (error) {
      if (error instanceof DeployError) throw error;
      throw new DeployError(`下载清单失败: ${error.message}`);
    } finally {
      clearTimeout(timer);
    }
  }
  let file = trimmed;
  if (trimmed.startsWith('file://')) file = new URL(trimmed).pathname;
  file = expandHome(file);
  const stat = await fs.stat(file).catch((error) => {
    throw new DeployError(`读取本地清单失败: ${file} (${error.message})`);
  });
  if (!stat.isFile()) throw new DeployError(`清单路径不是文件: ${file}`);
  if (stat.size > MANIFEST_MAX_BYTES) throw new DeployError('清单文件超过 5 MB 限制');
  return { content: await fs.readFile(file, 'utf8'), source: path.resolve(file) };
}

function warningsFor(summary, currentVersion) {
  const warnings = [];
  const required = summary.manifest.metadata.minToolVersion;
  if (required) {
    const parse = (v) => String(v).replace(/^v/i, '').split('.').map((part) => Number.parseInt(part, 10) || 0);
    const left = parse(currentVersion);
    const right = parse(required);
    const length = Math.max(left.length, right.length);
    let lower = false;
    for (let i = 0; i < length; i += 1) {
      if ((left[i] || 0) !== (right[i] || 0)) {
        lower = (left[i] || 0) < (right[i] || 0);
        break;
      }
    }
    if (lower) warnings.push(`清单要求工具版本不低于 ${required}，当前为 ${currentVersion}`);
  }
  if (summary.images.some((image) => image.sizeBytes === 0)) warnings.push('部分镜像未提供 sizeBytes，无法预估下载体积');
  if (!summary.manifest.spec.signature) warnings.push('清单未包含 signature，建议在正式发布时附加签名');
  if (!summary.manifest.spec.checksums) warnings.push('清单未包含 checksums，建议附加校验和文件');
  if (summary.images.every((image) => !image.platform)) warnings.push('清单未指定 platform，将按目标机默认架构拉取');
  return warnings;
}

export async function loadManifestSummary(source, options = {}) {
  const { content, source: normalizedSource } = await readSource(source);
  let parsed;
  try {
    parsed = YAML.parse(content);
  } catch (error) {
    throw new DeployError(`清单解析失败，请检查 YAML/JSON 格式: ${error.message}`);
  }
  const validated = validateManifest(parsed, options);
  const totalSizeBytes = validated.images.reduce((sum, image) => sum + image.sizeBytes, 0);
  const summary = {
    source: normalizedSource,
    manifest: validated.manifest,
    registry: validated.registry,
    namespace: validated.namespace,
    images: validated.images,
    totalSizeBytes,
    total_size_bytes: totalSizeBytes,
    warnings: []
  };
  summary.warnings = warningsFor(summary, await appVersion());
  return summary;
}

export function selectImages(summary, requested = []) {
  const names = requested.map((name) => String(name).trim()).filter(Boolean);
  if (names.length === 0) return summary.images;
  const selected = [];
  for (const name of names) {
    const image = summary.images.find((item) => item.name === name);
    if (!image) throw new DeployError(`清单中不存在镜像: ${name}`);
    selected.push(image);
  }
  return selected;
}

export async function writeManifest(file, manifest, options = {}) {
  const target = path.resolve(expandHome(file));
  const validated = validateManifest(manifest);
  const exists = await fs.access(target).then(() => true).catch(() => false);
  if (exists && options.overwrite !== true) {
    throw new DeployError(`清单文件已存在: ${target}`, { hint: '如需覆盖请传入 overwrite=true。' });
  }
  await fs.mkdir(path.dirname(target), { recursive: true });
  const content = YAML.stringify(validated.manifest);
  const temp = `${target}.${process.pid}.${Date.now()}.tmp`;
  await fs.writeFile(temp, content, 'utf8');
  await fs.rename(temp, target);
  return { path: target, images: validated.images };
}

export { MANIFEST_API_VERSION, MANIFEST_MAX_BYTES };
