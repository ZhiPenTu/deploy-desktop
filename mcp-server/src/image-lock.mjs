import fs from 'node:fs/promises';
import path from 'node:path';
import { DeployError } from './errors.mjs';
import { runRemote } from './ssh.mjs';
import { shellQuote } from './util.mjs';

export const IMAGE_LOCK_SCHEMA = 'deploy.desktop/image-lock/v1';
export const IMAGE_LOCK_ARTIFACT = 'image-lock';
export const DEFAULT_IMAGE_LOCK_PATH = 'deploy/images.lock.json';

const DIGEST = /^sha256:[0-9a-f]{64}$/;
const PLATFORM = /^[a-z0-9]+\/[a-z0-9_]+$/;

function fail(message, details) {
  return new DeployError(message, { code: 'IMAGE_LOCK_INVALID', details });
}

function platformOf(value) {
  const text = String(value || '').trim().toLowerCase();
  const [os, arch] = text.split('/');
  const normalized = arch === 'x86_64' ? 'amd64' : arch === 'aarch64' ? 'arm64' : arch;
  return os && normalized ? `${os}/${normalized}` : '';
}

function validatePlatformEntry(entry, label) {
  if (!entry || typeof entry !== 'object') throw fail(`${label} 必须是对象`);
  for (const key of ['manifest', 'imageId']) {
    const value = String(entry[key] || '').trim().toLowerCase();
    if (!DIGEST.test(value)) throw fail(`${label}.${key} 必须是 sha256 摘要`);
  }
  return { manifest: String(entry.manifest).trim(), imageId: String(entry.imageId).trim() };
}

export function validateImageLock(raw) {
  if (!raw || typeof raw !== 'object') throw fail('镜像锁必须是 JSON 对象');
  if (String(raw.schema || '').trim() !== IMAGE_LOCK_SCHEMA) {
    throw fail(`镜像锁格式必须是 ${IMAGE_LOCK_SCHEMA}`);
  }
  const requiredPlatforms = (Array.isArray(raw.requiredPlatforms) ? raw.requiredPlatforms : [])
    .map((item) => platformOf(item))
    .filter(Boolean);
  const images = (Array.isArray(raw.images) ? raw.images : []).map((item, index) => {
    const name = String(item?.name || '').trim();
    if (!name) throw fail(`images[${index}].name 不能为空`);
    const reference = String(item?.ref || '').trim();
    if (!reference) throw fail(`镜像 ${name} 缺少 ref`);
    const digest = String(item?.digest || '').trim().toLowerCase();
    if (!DIGEST.test(digest)) throw fail(`镜像 ${name} 的 digest 必须是 sha256 摘要`);
    const platforms = {};
    const declared = item?.platforms && typeof item.platforms === 'object' ? item.platforms : {};
    for (const [key, value] of Object.entries(declared)) {
      const platform = platformOf(key);
      if (!PLATFORM.test(platform)) throw fail(`镜像 ${name} 的平台标识无效：${key}`);
      platforms[platform] = validatePlatformEntry(value, `镜像 ${name} ${platform}`);
    }
    if (Object.keys(platforms).length === 0) throw fail(`镜像 ${name} 未声明任何平台`);
    return { name, ref: reference, digest, platforms };
  });
  for (const platform of requiredPlatforms) {
    for (const image of images) {
      if (!image.platforms[platform]) {
        throw fail(`镜像 ${image.name} 未声明 ${platform}，不满足交付包声明的双架构要求`);
      }
    }
  }
  return { schema: IMAGE_LOCK_SCHEMA, requiredPlatforms, images };
}

export function imageLockPath(bundle) {
  const artifact = bundle?.artifact?.(IMAGE_LOCK_ARTIFACT);
  const relative = String(artifact?.source || DEFAULT_IMAGE_LOCK_PATH).trim();
  return path.join(bundle.root, relative);
}

export async function loadImageLock(bundle) {
  const file = imageLockPath(bundle);
  const content = await fs.readFile(file, 'utf8').catch(() => null);
  if (content === null) {
    return { present: false, path: file, lock: null, warning: `交付包未附带镜像锁 ${path.basename(file)}，只能校验镜像存在性与架构` };
  }
  let raw;
  try {
    raw = JSON.parse(content);
  } catch (error) {
    throw fail(`镜像锁 ${file} 不是合法 JSON：${error.message}`, { path: file });
  }
  return { present: true, path: file, lock: validateImageLock(raw), warning: '' };
}

export function buildImageProbeScript(images) {
  const lines = [
    'HOST_PLATFORM=$(docker version --format \'{{.Server.Os}}/{{.Server.Arch}}\' 2>/dev/null || true)',
    'if [ -z "$HOST_PLATFORM" ]; then HOST_PLATFORM="$(uname -s | tr \'A-Z\' \'a-z\')/$(uname -m)"; fi',
    'printf \'HOST|%s\\n\' "$HOST_PLATFORM"'
  ];
  images.forEach((image, index) => {
    // containerd 存储下 tag 会解析到默认架构，digest 形式才解析到目标机架构，两种都探。
    const tag = String(image.imageRef).replace(/@sha256:[0-9a-f]+$/i, '');
    for (const [source, ref] of [['tag', tag], ['ref', image.imageRef]]) {
      lines.push(`PROBE=$(docker image inspect ${shellQuote(ref)} --format '{{.Os}}/{{.Architecture}}|{{.Id}}|{{join .RepoDigests ","}}' 2>/dev/null || true)`);
      lines.push(`printf 'IMG|%s|%s|%s\\n' ${shellQuote(String(index))} ${shellQuote(source)} "\${PROBE:-MISSING||}"`);
    }
  });
  return `${lines.join('\n')}\n`;
}

export function parseImageProbe(stdout) {
  let hostPlatform = '';
  const probes = [];
  for (const line of String(stdout || '').split('\n')) {
    const text = line.trim();
    if (text.startsWith('HOST|')) {
      hostPlatform = platformOf(text.slice(5));
      continue;
    }
    if (!text.startsWith('IMG|')) continue;
    const parts = text.split('|');
    const rawIndex = parts[1];
    const source = parts[2];
    const rest = parts.slice(3).join('|');
    const index = Number(rawIndex);
    if (!Number.isInteger(index)) continue;
    const [osArch, imageId = '', repoDigests = ''] = rest.split('|');
    const missing = osArch === 'MISSING';
    probes.push({
      index,
      source,
      osArch: missing ? '' : platformOf(osArch),
      imageId: missing ? '' : imageId.trim(),
      repoDigests: repoDigests.split(',').map((item) => item.trim()).filter(Boolean)
    });
  }
  return { hostPlatform, probes };
}

export function evaluateImagePrecheck({ lock, images, hostPlatform, probes }) {
  const problems = [];
  const warnings = [];
  const checks = [];
  if (!hostPlatform) problems.push('无法识别目标机 Docker 架构，请确认目标机 Docker 已运行');
  const locked = new Map((lock?.images || []).map((item) => [item.name, item]));
  const grouped = new Map();
  for (const item of probes || []) {
    if (!grouped.has(item.index)) grouped.set(item.index, []);
    grouped.get(item.index).push(item);
  }
  images.forEach((image, index) => {
    const candidates = (grouped.get(index) || []).filter((item) => item.imageId);
    const chosen = candidates.find((item) => hostPlatform && item.osArch === hostPlatform) || candidates[0] || null;
    const required = image.required !== false;
    const report = required ? problems : warnings;
    const check = {
      name: image.name,
      ref: image.imageRef,
      required,
      present: Boolean(chosen),
      osArch: chosen?.osArch || null,
      imageId: chosen?.imageId || null,
      expectedImageId: null,
      digest: null
    };
    checks.push(check);
    if (!chosen) {
      report.push(`目标机未导入镜像 ${image.name}（${image.imageRef}）`);
      return;
    }
    if (hostPlatform && !candidates.some((item) => item.osArch === hostPlatform)) {
      report.push(`镜像 ${image.name} 架构为 ${chosen.osArch}，目标机为 ${hostPlatform}`);
    }
    const entry = locked.get(image.name);
    if (!entry) {
      if (lock) warnings.push(`镜像锁未收录 ${image.name}，无法校验镜像身份`);
      return;
    }
    check.digest = entry.digest;
    const target = hostPlatform ? entry.platforms[hostPlatform] : null;
    if (!target) {
      report.push(`镜像锁未声明 ${image.name} 支持 ${hostPlatform || '目标机架构'}`);
      return;
    }
    check.expectedImageId = target.imageId;
    if (!candidates.some((item) => item.imageId === target.imageId)) {
      report.push(`镜像 ${image.name} 的内容与镜像锁不一致：${chosen.imageId} != ${target.imageId}`);
    }
    const digests = candidates.flatMap((item) => item.repoDigests);
    if (digests.length > 0 && !digests.some((item) => item.endsWith(`@${entry.digest}`))) {
      report.push(`镜像 ${image.name} 的 repo digest 与镜像锁不一致`);
    }
  });
  return { hostPlatform: hostPlatform || null, checks, problems, warnings };
}

export async function verifyImagesRemote(hostConfig, bundle) {
  const images = bundle.descriptor.images;
  if (images.length === 0) {
    return { skipped: true, hostPlatform: null, checks: [], problems: [], warnings: [], lockPath: '', lockPresent: false };
  }
  const loaded = await loadImageLock(bundle);
  const result = await runRemote(hostConfig, buildImageProbeScript(images), { timeoutMs: 120000 });
  const { hostPlatform, probes } = parseImageProbe(result.stdout);
  const evaluation = evaluateImagePrecheck({ lock: loaded.lock, images, hostPlatform, probes });
  return {
    skipped: false,
    ...evaluation,
    warnings: [loaded.warning, ...evaluation.warnings].filter(Boolean),
    lockPath: path.relative(bundle.root, loaded.path).split(path.sep).join('/'),
    lockPresent: loaded.present
  };
}
