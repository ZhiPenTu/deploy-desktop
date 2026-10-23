import fs from 'node:fs/promises';
import path from 'node:path';
import { DeployError } from './errors.mjs';
import { backupScript, applyScript } from './shell.mjs';
import { inspectRemoteDeployment } from './deploy.mjs';
import { Bundle, normalizeComponents as normalizeBundleComponents } from './bundle.mjs';
import { verifyImagesRemote } from './image-lock.mjs';
import { runRemote, uploadDir, uploadFile } from './ssh.mjs';
import { compareVersions, expandHome, shellQuote } from './util.mjs';

export const DEFAULT_UPDATE_ENDPOINT = 'https://github.com/ZhiPenTu/deploy-desktop/releases/latest/download/latest.json';

export function resolveUpdateEndpoint(endpoint) {
  const trimmed = String(endpoint || '').trim();
  return trimmed || DEFAULT_UPDATE_ENDPOINT;
}

export function normalizeComponents(bundle, input = []) {
  return normalizeBundleComponents(bundle, input);
}

export async function validateBundle(profile, bundleDirInput, components) {
  const root = path.resolve(expandHome(bundleDirInput));
  const stat = await fs.stat(root).catch(() => null);
  if (!stat?.isDirectory()) throw new DeployError(`更新包目录不存在: ${root}`);
  const bundle = await Bundle.load(root, { verify: true });
  if (bundle.product() !== profile.product) {
    throw new DeployError(`更新包属于「${bundle.product()}」，当前项目使用「${profile.product}」`, {
      code: 'PRODUCT_MISMATCH'
    });
  }
  const selected = bundle.selectComponents(components);
  const resolved = bundle.resolveArtifacts(selected, profile.installDir, profile.deployDir);
  const missing = [];
  for (const artifact of resolved) {
    const sourceStat = await fs.stat(artifact.source).catch(() => null);
    if (!sourceStat) {
      missing.push(`${artifact.key} (${artifact.label})`);
      continue;
    }
    if (artifact.kind === 'file' && (!sourceStat.isFile() || sourceStat.size === 0)) {
      missing.push(`${artifact.key} (${artifact.label})`);
      continue;
    }
    if (artifact.kind === 'dir') {
      if (!sourceStat.isDirectory()) { missing.push(`${artifact.key} (${artifact.label})`); continue; }
      const entries = await fs.readdir(artifact.source).catch(() => []);
      if (entries.length === 0) missing.push(`${artifact.key} (${artifact.label})`);
    }
  }
  if (missing.length > 0) {
    throw new DeployError(`更新包缺少所选组件所需的文件：${missing.join('、')}`, {
      code: 'BUNDLE_INCOMPLETE',
      hint: '请检查更新包是否完整，或重新从流水线下载制品。'
    });
  }
  return { bundle, artifacts: resolved };
}

function ensureRequiredContainers(bundle, components, status) {
  const required = [...new Set(bundle.descriptor.components
    .filter((component) => components.includes(component.key))
    .flatMap((component) => component.services))];
  const missing = required.filter((key) => {
    const container = status.containers.find((item) => item.serviceKey === key);
    return !container || container.state === 'not_created';
  });
  if (missing.length > 0) {
    throw new DeployError(`目标机缺少以下容器：${missing.join('、')}`, {
      code: 'CONTAINERS_MISSING',
      hint: '请先完成首次部署，或确认项目的部署目录是否正确。'
    });
  }
}

async function remoteFreeKb(hostConfig, dir) {
  const result = await runRemote(hostConfig, `df -Pk ${shellQuote(dir)} | awk 'NR==2{print $4}'\n`, { timeoutMs: 30000 });
  return Number(result.stdout.trim()) || 0;
}

export function buildBackupScript(installDir, artifacts) {
  return backupScript(installDir, artifacts.map((artifact) => artifact.target));
}

async function createRemoteBackup(hostConfig, installDir, artifacts) {
  const script = buildBackupScript(installDir, artifacts);
  const result = await runRemote(hostConfig, script, { timeoutMs: 30 * 60 * 1000 });
  const line = result.stdout.split('\n').find((item) => item.startsWith('BACKUP_DIR='));
  if (!line) throw new DeployError('备份完成但未返回备份目录');
  return line.slice('BACKUP_DIR='.length).trim();
}

async function applyRemoteUpdate(hostConfig, deployDir, components, bundle) {
  const selected = bundle.descriptor.components.filter((component) => components.includes(component.key));
  const composeSources = [bundle.descriptor.runtime.compose, bundle.descriptor.runtime.variant].filter(Boolean);
  const config = components.includes('image') || selected.some((component) => component.artifacts.some((key) =>
    composeSources.includes(bundle.descriptor.artifacts.find((artifact) => artifact.key === key)?.source)));
  const restart = [...new Set(selected.flatMap((component) => component.services))];
  const wait = config ? bundle.descriptor.runtime.services.map((service) => service.key) : restart;
  const source = bundle.descriptor.runtime.variant || bundle.descriptor.runtime.compose;
  const artifact = bundle.artifactForSource(source);
  const compose = artifact?.root === 'deploy' ? artifact.path : path.posix.basename(source);
  const script = applyScript({ deployDir,
    compose,
    config, restart, wait });
  const result = await runRemote(hostConfig, script, { timeoutMs: 30 * 60 * 1000 });
  return `${result.stdout}\n${result.stderr}`.trim();
}

export async function updateRemote(profile, hostConfig, options = {}) {
  const started = Date.now();
  const bundleDir = path.resolve(expandHome(options.bundleDir || profile.bundleDir));
  const progress = options.onProgress || (() => {});
  progress(3, '校验更新包');
  const loaded = await validateBundle(profile, bundleDir, options.components || []);
  const bundle = loaded.bundle;
  const components = normalizeComponents(bundle, options.components || []);
  const installDir = String(options.installDir || hostConfig.installDir || profile.installDir).replace(/\/+$/, '');
  const deployDir = [installDir, bundle.deploySourceRel()].filter(Boolean).join('/');
  const artifacts = bundle.resolveArtifacts(bundle.selectComponents(components), installDir, deployDir);
  progress(10, '检查目标机状态');
  const status = await inspectRemoteDeployment(profile, hostConfig, installDir);
  ensureRequiredContainers(bundle, components, status);
  progress(12, '校验基础镜像');
  const imageCheck = await verifyImagesRemote(hostConfig, bundle);
  if (imageCheck.problems.length > 0) {
    throw new DeployError(`基础镜像预检未通过：${imageCheck.problems.join('；')}`, {
      code: 'IMAGE_PRECHECK_FAILED',
      hint: '请先在镜像分发中上传并导入交付包声明的镜像，并确认目标机架构已被镜像锁声明支持。',
      details: { hostPlatform: imageCheck.hostPlatform, checks: imageCheck.checks }
    });
  }
  if (options.backup) {
    const freeKb = await remoteFreeKb(hostConfig, installDir);
    if (freeKb < 2 * 1024 * 1024) {
      throw new DeployError(`目标机剩余空间仅 ${(freeKb / 1024 / 1024).toFixed(1)} GB，更新备份至少需要 2 GB`, {
        code: 'INSUFFICIENT_DISK'
      });
    }
  }
  if (options.dryRun) {
    progress(100, '预检通过');
    return {
      success: true,
      dryRun: true,
      components,
      backupDir: null,
      artifacts: artifacts.map(({ key, label, root, path: relative, target }) => ({ key, label, root, path: relative, target })),
      images: imageCheck,
      output: `预检通过：目标机 ${installDir} 已部署，更新包、所需容器与 ${imageCheck.checks.length} 个基础镜像（目标机架构 ${imageCheck.hostPlatform || '未知'}）检查通过。未上传文件，也未重启服务。`,
      durationSecs: Math.round((Date.now() - started) / 1000),
      status
    };
  }

  let output = '';
  let backupDir = null;
  if (options.backup) {
    progress(14, '创建备份');
    backupDir = await createRemoteBackup(hostConfig, installDir, artifacts);
    output += `备份目录: ${backupDir}\n`;
    progress(20, '备份完成');
  }
  try {
    const total = artifacts.length;
    for (const [index, artifact] of artifacts.entries()) {
      const percent = 22 + Math.round(((index + 1) / Math.max(total, 1)) * 48);
      const label = artifact.label || artifact.key;
      progress(percent - 4, `上传${label}`);
      if (artifact.kind === 'file') {
        await uploadFile(hostConfig, artifact.source, artifact.target, { timeoutMs: 60 * 60 * 1000 });
        output += `已上传: ${artifact.target}\n`;
      } else {
        await uploadDir(hostConfig, artifact.source, artifact.target);
        output += `已替换: ${artifact.target}\n`;
      }
      progress(percent, `${label}已更新`);
    }
    progress(78, '应用更新');
    output += await applyRemoteUpdate(hostConfig, deployDir, components, bundle);
  } catch (error) {
    throw new DeployError(error.message || '更新未完成，请检查目标状态', {
      code: error.code || 'UPDATE_FAILED',
      hint: '请检查已更新组件与备份，确认目标状态后再重试。',
      details: { ...error.details, backupDir, output }
    });
  }
  progress(94, '服务已重启');
  const finalStatus = await inspectRemoteDeployment(profile, hostConfig, installDir).catch(() => null);
  progress(100, '更新完成');
  return {
    success: true,
    dryRun: false,
    components,
    backupDir,
    artifacts: artifacts.map(({ key, label, root, path: relative, target }) => ({ key, label, root, path: relative, target })),
    images: imageCheck,
    output,
    durationSecs: Math.round((Date.now() - started) / 1000),
    status: finalStatus
  };
}

function platformKey() {
  if (process.platform === 'darwin') return process.arch === 'arm64' ? 'darwin-aarch64' : 'darwin-x86_64';
  if (process.platform === 'win32') return 'windows-x86_64';
  if (process.platform === 'linux') return process.arch === 'arm64' ? 'linux-aarch64' : 'linux-x86_64';
  return `${process.platform}-${process.arch}`;
}

export async function checkAppUpdate(options = {}) {
  const endpoint = resolveUpdateEndpoint(options.endpoint);
  if (!/^https:\/\//i.test(endpoint) && !options.allowInsecure) {
    throw new DeployError('更新源必须使用 HTTPS；内网 HTTP 镜像需显式传入 allowInsecure=true');
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 30000);
  let payload;
  try {
    const response = await fetch(endpoint, { signal: controller.signal, headers: { 'User-Agent': 'deploy-desktop-mcp/0.1.2' } });
    if (!response.ok) throw new DeployError(`读取更新源失败: HTTP ${response.status}`);
    payload = await response.json();
  } finally {
    clearTimeout(timer);
  }
  const currentVersion = options.currentVersion || '0.0.0';
  const latestVersion = String(payload.version || '').trim();
  if (!latestVersion) throw new DeployError('更新源缺少 version 字段');
  const key = platformKey();
  const platform = payload.platforms?.[key] || null;
  return {
    currentVersion,
    latestVersion,
    available: compareVersions(currentVersion, latestVersion) < 0,
    notes: payload.notes || '',
    pubDate: payload.pub_date || null,
    platform: key,
    url: platform?.url || null,
    signature: platform?.signature ? 'present' : null,
    message: compareVersions(currentVersion, latestVersion) < 0
      ? `发现新版本 ${latestVersion}（当前 ${currentVersion}）`
      : `当前已是最新版本 ${currentVersion}`,
    installHint: '桌面应用更新请在 deploy-desktop 界面中使用“检查更新”，MCP 服务不会自动替换正在运行的应用。'
  };
}
