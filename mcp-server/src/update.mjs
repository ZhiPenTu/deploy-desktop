import fs from 'node:fs/promises';
import path from 'node:path';
import { DeployError } from './errors.mjs';
import { run } from './exec.mjs';
import { inspectRemoteDeployment } from './deploy.mjs';
import { serviceKeysForComponent } from './profile.mjs';
import { runRemote, uploadDir, uploadFile } from './ssh.mjs';
import { compareVersions, expandHome, formatBytes, nowStamp, shellQuote } from './util.mjs';

const COMPONENTS = ['backend', 'gateway', 'frontend', 'config'];

export const DEFAULT_UPDATE_ENDPOINT = 'https://github.com/ZhiPenTu/deploy-desktop/releases/latest/download/latest.json';

export function resolveUpdateEndpoint(endpoint) {
  const trimmed = String(endpoint || '').trim();
  return trimmed || DEFAULT_UPDATE_ENDPOINT;
}

export function normalizeComponents(input = []) {
  const result = [];
  for (const item of input) {
    const key = String(item).trim().toLowerCase();
    if (!COMPONENTS.includes(key)) throw new DeployError(`不支持的更新组件: ${key}`, { hint: `可用: ${COMPONENTS.join(', ')}` });
    if (!result.includes(key)) result.push(key);
  }
  if (result.length === 0) throw new DeployError('请至少选择一个更新组件');
  return result;
}

export async function validateBundle(profile, bundleDirInput, components) {
  const root = path.resolve(expandHome(bundleDirInput));
  const app = path.join(root, 'app');
  const deploy = path.join(root, 'deploy');
  const paths = {
    serverJar: null,
    gatewayClasses: null,
    gatewayLib: null,
    frontendDist: null,
    composeFile: null,
    nginxDir: null,
    oceanbaseDir: null
  };
  const missing = [];
  const rootStat = await fs.stat(root).catch(() => null);
  if (!rootStat?.isDirectory()) throw new DeployError(`更新包目录不存在: ${root}`);
  if (components.includes('backend')) {
    const jar = path.join(app, profile.backendArtifact);
    const stat = await fs.stat(jar).catch(() => null);
    if (stat?.isFile() && stat.size > 0) paths.serverJar = jar;
    else missing.push(`app/${profile.backendArtifact}`);
  }
  if (components.includes('gateway')) {
    const classes = path.join(app, 'gateway-classes');
    if ((await fs.stat(classes).catch(() => null))?.isDirectory()) paths.gatewayClasses = classes;
    else missing.push('app/gateway-classes/');
    const lib = path.join(app, 'gateway-lib');
    if ((await fs.stat(lib).catch(() => null))?.isDirectory()) paths.gatewayLib = lib;
  }
  if (components.includes('frontend')) {
    const dist = path.join(app, 'frontend-dist');
    if (await fs.access(path.join(dist, 'index.html')).then(() => true).catch(() => false)) paths.frontendDist = dist;
    else missing.push('app/frontend-dist/index.html');
  }
  if (components.includes('config')) {
    const compose = path.join(deploy, 'docker-compose.yml');
    if (await fs.access(compose).then(() => true).catch(() => false)) paths.composeFile = compose;
    else missing.push('deploy/docker-compose.yml');
    if ((await fs.stat(path.join(deploy, 'nginx')).catch(() => null))?.isDirectory()) paths.nginxDir = path.join(deploy, 'nginx');
    if ((await fs.stat(path.join(deploy, 'oceanbase')).catch(() => null))?.isDirectory()) paths.oceanbaseDir = path.join(deploy, 'oceanbase');
  }
  if (missing.length > 0) {
    throw new DeployError(`更新包缺少所选组件所需的文件：${missing.join('、')}`, {
      code: 'BUNDLE_INCOMPLETE',
      hint: '请检查更新目录是否包含完整的 app/ 与 deploy/ 内容。'
    });
  }
  return paths;
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
      hint: '请先完成首次部署，或确认部署目录与 profile 是否匹配。'
    });
  }
}

async function remoteFreeKb(hostConfig, dir) {
  const result = await runRemote(hostConfig, `df -Pk ${shellQuote(dir)} | awk 'NR==2{print $4}'\n`, { timeoutMs: 30000 });
  return Number(result.stdout.trim()) || 0;
}

async function createRemoteBackup(profile, hostConfig, installDir, deployDir) {
  const script = `set -e
INSTALL=${shellQuote(installDir)}
DEPLOY=${shellQuote(deployDir)}
STAMP=$(date +%Y%m%d-%H%M%S)
BACKUP="$INSTALL/backup/update-$STAMP"
mkdir -p "$BACKUP"
if [ -f "$INSTALL/backend-java/server/target/${profile.backendArtifact}" ]; then
  mkdir -p "$BACKUP/server"
  cp -a "$INSTALL/backend-java/server/target/${profile.backendArtifact}" "$BACKUP/server/"
fi
if [ -d "$INSTALL/backend-java/gateway-app/target/classes" ]; then
  mkdir -p "$BACKUP/gateway"
  cp -a "$INSTALL/backend-java/gateway-app/target/classes" "$BACKUP/gateway/"
fi
if [ -d "$INSTALL/backend-java/gateway-app/target/lib" ]; then
  mkdir -p "$BACKUP/gateway"
  cp -a "$INSTALL/backend-java/gateway-app/target/lib" "$BACKUP/gateway/"
fi
if [ -d "$DEPLOY/frontend_dist" ]; then
  mkdir -p "$BACKUP/frontend"
  cp -a "$DEPLOY/frontend_dist" "$BACKUP/frontend/"
fi
if [ -f "$DEPLOY/docker-compose.yml" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/docker-compose.yml" "$BACKUP/deploy/"
fi
if [ -d "$DEPLOY/nginx" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/nginx" "$BACKUP/deploy/"
fi
if [ -d "$DEPLOY/oceanbase" ]; then
  mkdir -p "$BACKUP/deploy"
  cp -a "$DEPLOY/oceanbase" "$BACKUP/deploy/"
fi
echo "BACKUP_DIR=$BACKUP"
`;
  const result = await runRemote(hostConfig, script, { timeoutMs: 30 * 60 * 1000 });
  const line = result.stdout.split('\n').find((item) => item.startsWith('BACKUP_DIR='));
  if (!line) throw new DeployError('备份完成但未返回备份目录');
  return line.slice('BACKUP_DIR='.length).trim();
}

async function applyRemoteUpdate(profile, hostConfig, deployDir, components) {
  const restartServices = [];
  if (components.includes('backend')) restartServices.push(...serviceKeysForComponent(profile, 'backend'));
  if (components.includes('gateway')) restartServices.push(...serviceKeysForComponent(profile, 'gateway'));
  if (components.includes('frontend')) restartServices.push(...serviceKeysForComponent(profile, 'frontend'));
  const waitServices = components.includes('config')
    ? profile.containers.map((container) => container.serviceKey)
    : [...new Set(restartServices)];
  const frontendContainer = `${profile.containerPrefix}-frontend`;
  let script = `set -euo pipefail
DEPLOY=${shellQuote(deployDir)}
cd "$DEPLOY"
compose() {
  if docker compose version >/dev/null 2>&1; then docker compose "$@"; else docker-compose "$@"; fi
}
`;
  if (components.includes('config')) {
    script += 'echo "==> 应用部署配置"\ncompose up -d\n';
  }
  if (components.includes('frontend')) {
    script += `echo "==> 更新前端静态页面"
if [ "$(docker inspect --format '{{.State.Status}}' ${shellQuote(frontendContainer)} 2>/dev/null || echo missing)" != "running" ]; then
  compose up -d frontend
fi
docker cp "$DEPLOY/frontend_dist/." ${shellQuote(`${frontendContainer}:/usr/share/nginx/html/`)}
`;
  }
  if (restartServices.length > 0) {
    script += `echo "==> 重启服务: ${restartServices.join(' ')}"\ncompose restart ${restartServices.join(' ')}\n`;
  }
  if (waitServices.length > 0) {
    script += 'echo "==> 等待服务健康检查"\n';
    for (const service of waitServices) {
      const container = `${profile.containerPrefix}-${service}`;
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
  const components = normalizeComponents(options.components || []);
  const bundleDir = path.resolve(expandHome(options.bundleDir || ''));
  const installDir = String(options.installDir || hostConfig.installDir || profile.installDir).replace(/\/+$/, '');
  const deployDir = `${installDir}/deploy`;
  const progress = options.onProgress || (() => {});
  progress(3, '校验更新包');
  const bundle = await validateBundle(profile, bundleDir, components);
  progress(10, '检查目标机状态');
  const status = await inspectRemoteDeployment(profile, hostConfig, installDir);
  ensureRequiredContainers(profile, components, status);
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
      output: `预检通过：目标机 ${installDir} 已部署，更新包与所需容器检查通过。未上传文件，也未重启服务。`,
      durationSecs: Math.round((Date.now() - started) / 1000),
      status
    };
  }

  let output = '';
  let backupDir = null;
  if (options.backup) {
    progress(15, '创建备份');
    backupDir = await createRemoteBackup(profile, hostConfig, installDir, deployDir);
    output += `备份目录: ${backupDir}\n`;
    progress(20, '备份完成');
  }
  if (bundle.serverJar) {
    progress(28, '上传后端服务');
    const remote = `${installDir}/backend-java/server/target/${profile.backendArtifact}`;
    await uploadFile(hostConfig, bundle.serverJar, remote, { timeoutMs: 60 * 60 * 1000 });
    output += `已上传: ${remote}\n`;
    progress(38, '后端服务已上传');
  }
  if (bundle.gatewayClasses) {
    progress(42, '上传网关 classes');
    const remote = `${installDir}/backend-java/gateway-app/target/classes`;
    await uploadDir(hostConfig, bundle.gatewayClasses, remote);
    output += `已替换: ${remote}\n`;
    progress(48, '网关 classes 已替换');
  }
  if (bundle.gatewayLib) {
    progress(50, '上传网关依赖');
    const remote = `${installDir}/backend-java/gateway-app/target/lib`;
    await uploadDir(hostConfig, bundle.gatewayLib, remote);
    output += `已替换: ${remote}\n`;
    progress(55, '网关 lib 已替换');
  }
  if (bundle.frontendDist) {
    progress(58, '上传前端页面');
    const remote = `${deployDir}/frontend_dist`;
    await uploadDir(hostConfig, bundle.frontendDist, remote);
    output += `已替换: ${remote}\n`;
    progress(64, '前端 dist 已替换');
  }
  if (bundle.composeFile) {
    progress(66, '上传部署配置');
    await uploadFile(hostConfig, bundle.composeFile, `${deployDir}/docker-compose.yml`, { timeoutMs: 10 * 60 * 1000 });
    if (bundle.nginxDir) await uploadDir(hostConfig, bundle.nginxDir, `${deployDir}/nginx`);
    if (bundle.oceanbaseDir) await uploadDir(hostConfig, bundle.oceanbaseDir, `${deployDir}/oceanbase`);
    output += `已更新部署配置: ${deployDir}\n`;
    progress(72, '部署配置已更新');
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
