import fs from 'node:fs/promises';
import path from 'node:path';
import { DeployError } from './errors.mjs';
import { run } from './exec.mjs';
import { inspectRemoteDeployment } from './deploy.mjs';
import { Bundle, normalizeComponents as normalizeBundleComponents } from './bundle.mjs';
import { verifyImagesRemote } from './image-lock.mjs';
import { containerName, serviceKeysForComponent } from './profile.mjs';
import { runRemote, uploadDir, uploadFile } from './ssh.mjs';
import { compareVersions, expandHome, formatBytes, nowStamp, shellQuote } from './util.mjs';

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

function requiredServiceKeys(profile, components) {
  const keys = [];
  for (const component of components) {
    keys.push(...serviceKeysForComponent(profile, component));
  }
  return [...new Set(keys)];
}

function ensureRequiredContainers(profile, components, status) {
  const required = requiredServiceKeys(profile, components);
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
  const lines = artifacts.map((artifact) => {
    const target = shellQuote(artifact.target);
    const relative = shellQuote(path.posix.relative(installDir, artifact.target));
    return `if [ -e ${target} ]; then
  REL=${relative}
  mkdir -p "$BACKUP/$(dirname "$REL")"
  cp -a ${target} "$BACKUP/$REL"
fi`;
  }).join('\n');
  return `set -e
INSTALL=${shellQuote(installDir)}
STAMP=$(date +%Y%m%d-%H%M%S)
BACKUP="$INSTALL/backup/update-$STAMP"
mkdir -p "$BACKUP"
${lines}
echo "BACKUP_DIR=$BACKUP"
`;
}

async function createRemoteBackup(profile, hostConfig, installDir, deployDir, artifacts) {
  const script = buildBackupScript(installDir, artifacts);
  const result = await runRemote(hostConfig, script, { timeoutMs: 30 * 60 * 1000 });
  const line = result.stdout.split('\n').find((item) => item.startsWith('BACKUP_DIR='));
  if (!line) throw new DeployError('备份完成但未返回备份目录');
  return line.slice('BACKUP_DIR='.length).trim();
}

async function applyRemoteUpdate(profile, hostConfig, deployDir, components) {
  const restartServices = [];
  for (const component of components) {
    if (component === 'config' || component === 'image') continue;
    restartServices.push(...serviceKeysForComponent(profile, component));
  }
  const waitServices = components.includes('config')
    ? profile.containers.map((container) => container.serviceKey)
    : [...new Set(restartServices)];
  const composeMain = profile.compose.fileRelative;
  const composeVariant = profile.compose.variantRelative;
  let script = `set -euo pipefail
DEPLOY=${shellQuote(deployDir)}
cd "$DEPLOY"
COMPOSE_MAIN=${shellQuote(composeMain)}
COMPOSE_VARIANT=${shellQuote(composeVariant)}
if [ -n "$COMPOSE_VARIANT" ] && [ -f "$COMPOSE_VARIANT" ]; then COMPOSE_FILE="$COMPOSE_VARIANT"
elif [ -f "$COMPOSE_MAIN" ]; then COMPOSE_FILE="$COMPOSE_MAIN"
else echo "未找到 Compose 清单: $COMPOSE_MAIN" >&2; exit 3; fi
compose() {
  if docker compose version >/dev/null 2>&1; then docker compose -f "$COMPOSE_FILE" "$@"; else docker-compose -f "$COMPOSE_FILE" "$@"; fi
}
`;
  if (components.includes('config')) {
    script += 'echo "==> 应用部署配置"\ncompose up -d\n';
  }
  if (restartServices.length > 0) {
    script += `echo "==> 重启服务: ${restartServices.join(' ')}"\ncompose restart ${restartServices.join(' ')}\n`;
  }
  if (waitServices.length > 0) {
    script += 'echo "==> 等待服务健康检查"\n';
    for (const service of waitServices) {
      const container = containerName(profile, service);
      script += `ready=no
for i in $(seq 1 45); do
  health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' ${shellQuote(container)} 2>/dev/null || echo missing)
  state=$(docker inspect --format '{{.State.Status}}' ${shellQuote(container)} 2>/dev/null || echo missing)
  if [ "$health" = "healthy" ] || { [ "$health" = "none" ] && [ "$state" = "running" ]; }; then ready=yes; break; fi
  sleep 2
done
if [ "$ready" != "yes" ]; then
  echo "服务 ${container} 未就绪 (health=$health, state=$state)" >&2
  docker logs --tail 40 ${shellQuote(container)} 2>&1 || true
  exit 4
fi
`;
    }
  }
  script += 'echo "==> 当前容器状态"\ncompose ps\n';
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
  ensureRequiredContainers(profile, components, status);
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
    backupDir = await createRemoteBackup(profile, hostConfig, installDir, deployDir, artifacts);
    output += `备份目录: ${backupDir}\n`;
    progress(20, '备份完成');
  }
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
  output += await applyRemoteUpdate(profile, hostConfig, deployDir, components);
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
