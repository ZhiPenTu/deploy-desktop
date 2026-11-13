import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';
import { Bundle, isInternalDeployPath } from './bundle.mjs';
import { appVersion } from './config.mjs';
import { DeployError } from './errors.mjs';
import { verifyImagesRemote } from './image-lock.mjs';
import { activeProject, appConfigDir, validateInstallDir } from './projects.mjs';
import { deploymentBaselineScript, versionDeployLaunchScript, versionDeployScript } from './shell.mjs';
import { runRemote, uploadDir, uploadFile } from './ssh.mjs';
import { expandHome, shellQuote } from './util.mjs';

const TERMINAL_STATES = new Set(['committed', 'recovered', 'recovery_failed', 'failed']);
const BLOCKING_STATES = new Set(['pending', 'running', 'failed', 'recovering', 'recovery_failed']);

function decodeBase64(value) {
  return Buffer.from(String(value || '').trim(), 'base64');
}
export function redactDiagnostics(value) {
  return String(value || '')
    .replace(/-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g, '[私钥已隐藏]')
    .replace(/\b(?:https?|ssh):\/\/[^\s]+/gi, '[地址已隐藏]')
    .replace(/((?:["']?)[\w-]*(?:password|passwd|token|secret|authorization)["']?\s*[:=]\s*)(?:"(?:\\.|[^"])*"|'(?:\\.|[^'])*'|[^\s,;]+)/gi, '$1[已隐藏]')
    .replace(/((?:--password|--token)\s+)(?:"[^"]*"|'[^']*'|\S+)/gi, '$1[已隐藏]')
    .replace(/(Bearer\s+)[\w.-]+/gi, '$1[已隐藏]');
}

function encodeBase64(value) {
  return Buffer.from(value).toString('base64');
}

function trimSlashes(value) {
  return String(value || '').replace(/\/+$/, '');
}

function remoteJoin(base, child) {
  const left = trimSlashes(base);
  const right = String(child || '').trim().replace(/^\/+/, '');
  if (!right) return left;
  if (!left) return right;
  return `${left}/${right}`;
}

function relativeInstallPath(install, target) {
  const relative = path.posix.relative(trimSlashes(install), String(target).replaceAll('\\', '/'));
  if (!relative || relative.startsWith('..') || path.posix.isAbsolute(relative)) {
    throw new DeployError('部署事务路径超出部署目录', { code: 'TRANSACTION_PATH_INVALID' });
  }
  validateManagedRelative(relative, { allowRecord: false });
  return relative;
}

function validateManagedRelative(value, { allowRecord = true } = {}) {
  const relative = String(value || '').trim().replaceAll('\\', '/');
  const parts = relative.split('/');
  if (!relative
    || path.posix.isAbsolute(relative)
    || parts.some((part) => part === '' || part === '.' || part === '..')) {
    throw new DeployError('部署事务路径无效', { code: 'TRANSACTION_PATH_INVALID' });
  }
  const internal = isInternalDeployPath(relative);
  if (internal && !(allowRecord && relative === '.deploy-desktop/deployed.json')) {
    throw new DeployError('部署事务路径指向内部状态目录', { code: 'TRANSACTION_PATH_INVALID' });
  }
  return relative;
}

function projectEnvPath(projectId) {
  return path.join(appConfigDir(), 'project-envs', `${projectId}.env`);
}

async function readProjectEnv(projectId) {
  if (!projectId) return '';
  try {
    return await fs.readFile(projectEnvPath(projectId), 'utf8');
  } catch (error) {
    if (error?.code === 'ENOENT') return '';
    throw new DeployError('无法读取项目的环境配置，请检查文件权限后重试', {
      code: 'PROJECT_ENV_UNREADABLE',
      details: { code: error?.code || '' }
    });
  }
}

export function parseTransactionStatus(raw, transaction) {
  const values = new Map();
  for (const line of String(raw || '').split('\n')) {
    const index = line.indexOf('=');
    if (index < 0) continue;
    values.set(line.slice(0, index).trim(), line.slice(index + 1).trim());
  }
  if (values.get('phase')) transaction.phase = values.get('phase');
  if (values.get('state')) transaction.state = values.get('state');
  if (values.get('updatedAt')) transaction.updatedAt = values.get('updatedAt');
  if (values.has('deploymentStarted')) transaction.deploymentStarted = values.get('deploymentStarted') === 'yes';
  const error = values.get('errorBase64');
  if (error) transaction.error = decodeBase64(error).toString('utf8');
  const recoveryError = values.get('recoveryErrorBase64');
  if (recoveryError) transaction.recoveryError = decodeBase64(recoveryError).toString('utf8');
  return transaction;
}

function normalizeTransaction(raw) {
  return {
    schemaVersion: Number(raw?.schemaVersion) || 1,
    id: String(raw?.id || ''),
    product: String(raw?.product || ''),
    version: String(raw?.version || ''),
    previousVersion: String(raw?.previousVersion || ''),
    installDir: String(raw?.installDir || ''),
    deployDir: String(raw?.deployDir || ''),
    phase: String(raw?.phase || 'preparing'),
    state: String(raw?.state || 'pending'),
    startedAt: String(raw?.startedAt || ''),
    updatedAt: String(raw?.updatedAt || ''),
    snapshotDir: String(raw?.snapshotDir || ''),
    hadDeployment: Boolean(raw?.hadDeployment),
    deploymentStarted: Boolean(raw?.deploymentStarted),
    projectName: String(raw?.projectName || ''),
    previousCompose: String(raw?.previousCompose || ''),
    newCompose: String(raw?.newCompose || ''),
    files: Array.isArray(raw?.files) ? raw.files : [],
    volumes: Array.isArray(raw?.volumes) ? raw.volumes : [],
    oldImages: Array.isArray(raw?.oldImages) ? raw.oldImages : [],
    services: Array.isArray(raw?.services) ? raw.services.map(String) : [],
    previousServices: Array.isArray(raw?.previousServices) ? raw.previousServices.map(String) : [],
    frontendService: String(raw?.frontendService || ''),
    frontendPort: raw?.frontendPort ? Number(raw.frontendPort) : null,
    error: raw?.error ? String(raw.error) : null,
    recoveryError: raw?.recoveryError ? String(raw.recoveryError) : null
  };
}

export async function inspectRemoteTransaction(hostConfig, installDir) {
  const install = trimSlashes(validateInstallDir(installDir));
  const root = `${install}/.deploy-desktop/transactions`;
  const result = await runRemote(hostConfig, `set -e
ROOT=${shellQuote(root)}
[ -d "$ROOT" ] || exit 0
find "$ROOT" -mindepth 1 -maxdepth 1 -type d -name 'deploy-*' | sort -r | while IFS= read -r dir; do
  if [ ! -f "$dir/journal.json" ]; then
    [ ! -f "$dir/status.env" ] || { echo '部署事务缺少 journal 但存在运行状态，请检查现场后再部署' >&2; exit 4; }
    continue
  fi
  printf 'JOURNAL_B64='
  base64 < "$dir/journal.json" | tr -d '\\n'
  printf '\\n'
  if [ -f "$dir/status.env" ]; then
    printf 'STATUS_B64='
    base64 < "$dir/status.env" | tr -d '\\n'
    printf '\\n'
  fi
done
`, { timeoutMs: 120000 });
  const lines = result.stdout.split('\n');
  const transactions = [];
  try {
    for (const line of lines) {
      if (line.startsWith('JOURNAL_B64=')) {
        transactions.push(normalizeTransaction(JSON.parse(decodeBase64(line.slice(12)).toString('utf8'))));
      } else if (line.startsWith('STATUS_B64=') && transactions.length) {
        parseTransactionStatus(decodeBase64(line.slice(11)).toString('utf8'), transactions.at(-1));
      }
    }
  } catch {
    throw new DeployError('目标机部署事务记录格式无效', { code: 'TRANSACTION_INVALID' });
  }
  return transactions.find((transaction) => !['committed', 'recovered'].includes(transaction.state))
    || transactions[0] || null;
}

export function assertNoBlockingTransaction(transaction) {
  if (!transaction || ['committed', 'recovered'].includes(transaction.state)) return;
  if (transaction.state === 'recovery_failed') {
    throw new DeployError(
      `检测到恢复失败的部署事务 ${transaction.id}，现场保留在 ${transaction.snapshotDir}，请人工处理后再部署`,
      { code: 'RECOVERY_FAILED_BLOCKING' }
    );
  }
  if (transaction.state === 'recovering') {
    throw new DeployError('该事务正在恢复或上次恢复被中断，请先检查事务现场后再处理', {
      code: 'RECOVERY_IN_PROGRESS'
    });
  }
  throw new DeployError(
    `目标机存在未完成部署事务 ${transaction.id}（阶段：${transaction.phase}），请先恢复上一版本`,
    { code: 'TRANSACTION_INCOMPLETE' }
  );
}

export function assertFullComponents(bundle, components) {
  const expected = bundle.descriptor.components.map((component) => component.key);
  const actual = [...new Set((Array.isArray(components) ? components : []).map(String))];
  const missing = expected.filter((key) => !actual.includes(key));
  const extra = actual.filter((key) => !expected.includes(key));
  if (missing.length > 0 || extra.length > 0) {
    throw new DeployError(
      '当前版本已改为完整版本部署，不支持部分组件更新。请省略 components 或传入全部组件',
      {
        code: 'FULL_DEPLOY_REQUIRED',
        details: { missing, extra, expected }
      }
    );
  }
  return expected;
}

async function remoteReadRecord(hostConfig, installDir) {
  const file = `${trimSlashes(installDir)}/.deploy-desktop/deployed.json`;
  const result = await runRemote(hostConfig, `set -eo pipefail
if [ -e ${shellQuote(file)} ]; then
  [ -f ${shellQuote(file)} ] && [ -s ${shellQuote(file)} ] || exit 4
  base64 < ${shellQuote(file)} | tr -d '\\n'
fi`, {
    timeoutMs: 60000
  });
  const encoded = result.stdout.trim();
  if (!encoded) return null;
  try {
    const record = JSON.parse(decodeBase64(encoded).toString('utf8'));
    if (![1, 2].includes(record.schemaVersion) || !record.product || !record.version || !Array.isArray(record.artifacts)) throw new Error();
    return record;
  } catch {
    throw new DeployError('上一版本部署记录格式无效，请确认旧部署状态后再部署', {
      code: 'DEPLOYED_RECORD_INVALID'
    });
  }
}

async function findOldCompose(hostConfig, installDir, deployDir, record) {
  const verify = async (file, stored = file) => {
    resolveStoredEffectiveCompose(installDir, deployDir, stored, 'FOUND');
    const checked = await runRemote(hostConfig, `if [ ! -e ${shellQuote(file)} ]; then
  printf 'MISSING\\n'
elif [ ! -f ${shellQuote(file)} ]; then
  printf 'INVALID\\n'
else
  base=$(readlink -f -- ${shellQuote(deployDir)} 2>/dev/null) || { printf 'INVALID\\n'; exit 0; }
  target=$(readlink -f -- ${shellQuote(file)} 2>/dev/null) || { printf 'INVALID\\n'; exit 0; }
  case "$target" in
    "$base"/*) printf 'FOUND\\n' ;;
    *) printf 'OUTSIDE\\n' ;;
  esac
fi`, { timeoutMs: 60000 });
    return resolveStoredEffectiveCompose(installDir, deployDir, stored, checked.stdout.trim());
  };
  if (record?.effectiveCompose) {
    const file = path.posix.isAbsolute(record.effectiveCompose)
      ? record.effectiveCompose
      : remoteJoin(installDir, record.effectiveCompose);
    return verify(file, record.effectiveCompose);
  }
  const active = await runRemote(hostConfig, `docker ps -a --filter ${shellQuote(`label=com.docker.compose.project.working_dir=${deployDir}`)} --format '{{.Label "com.docker.compose.project.config_files"}}'`, { timeoutMs: 60000 });
  if (active.stdout.split(/[\n,]/).map((file) => file.trim()).filter(Boolean).some((file) =>
    !file.startsWith(`${trimSlashes(deployDir)}/`) && !file.startsWith(`${trimSlashes(installDir)}/.deploy-desktop/`))) {
    throw new DeployError('上一版本引用部署目录外的 Compose 清单，本期无法安全恢复', { code: 'OLD_COMPOSE_UNSUPPORTED' });
  }
  const actual = [...new Set(active.stdout.split(/[\n,]/).map((file) => file.trim())
    .filter((file) => file.startsWith(`${trimSlashes(deployDir)}/`)))];
  if (actual.length > 1) {
    throw new DeployError('上一版本合并使用多份 Compose 清单，本期无法安全恢复，请先确认生效配置', { code: 'OLD_COMPOSE_AMBIGUOUS' });
  }
  if (actual.length === 1) return verify(actual[0]);
  const candidates = [];
  const fullPaths = [];
  for (const compose of record?.composeFiles || []) {
    const stored = String(compose.path || '').replaceAll('\\', '/');
    if (!stored) continue;
    if (stored.startsWith(`${trimSlashes(installDir)}/`)) {
      fullPaths.push(stored);
    } else if (!path.posix.isAbsolute(stored)) {
      const deployRelative = path.posix.relative(installDir, deployDir);
      candidates.push(stored.startsWith(`${deployRelative}/`) ? stored.slice(deployRelative.length + 1) : stored);
    }
  }
  for (const artifact of record?.artifacts || []) {
    if (!String(artifact.path || '').endsWith('.yml') && !String(artifact.path || '').endsWith('.yaml')) continue;
    const relative = artifact.root === 'install' ? remoteJoin(installDir, artifact.path) : artifact.path;
    if (path.posix.isAbsolute(relative)) {
      fullPaths.push(relative);
      continue;
    }
    if (relative && !candidates.includes(relative)) candidates.push(relative);
  }
  for (const name of ['docker-compose.lan.yml', 'docker-compose.yml', 'docker-compose.yaml', 'compose.yaml', 'compose.yml']) {
    if (!candidates.includes(name)) candidates.push(name);
  }
  const candidatesToCheck = [...new Map([
    ...fullPaths.map((candidate) => ({ label: path.posix.relative(deployDir, candidate), path: candidate })),
    ...candidates.map((candidate) => ({ label: candidate, path: remoteJoin(deployDir, candidate) }))
  ].map((item) => [item.path, item])).values()];
  const checks = candidatesToCheck
    .map((candidate) => `if [ -f ${shellQuote(candidate.path)} ]; then printf '%s\\n' ${shellQuote(candidate.path)}; fi`)
    .join('\n');
  const result = await runRemote(hostConfig, `set -e\n${checks}\nexit 0\n`, { timeoutMs: 60000 });
  const existing = result.stdout.split('\n').map((file) => file.trim()).filter(Boolean);
  if (existing.length > 1) {
    throw new DeployError('旧部署未记录实际生效的 Compose，且存在多份清单；请先确认旧版本的生效清单', { code: 'OLD_COMPOSE_AMBIGUOUS' });
  }
  if (existing[0]) return verify(existing[0]);
  if (record) throw new DeployError('旧部署记录存在，但找不到上一版本 Compose 清单，已停止部署', { code: 'OLD_COMPOSE_MISSING' });
  return '';
}

export function resolveStoredEffectiveCompose(installDir, deployDir, storedEffective, marker) {
  if (storedEffective.includes('\\') || storedEffective.split('/').includes('..')) {
    throw new DeployError('上一版本 Compose 路径包含无效跳转，已停止部署', {
      code: 'OLD_COMPOSE_UNSUPPORTED'
    });
  }
  const file = path.posix.isAbsolute(storedEffective)
    ? storedEffective
    : remoteJoin(installDir, storedEffective);
  if (!file.startsWith(`${trimSlashes(deployDir)}/`)) {
    throw new DeployError('上一版本 Compose 位于其他目录，本期不支持迁移部署目录', {
      code: 'OLD_COMPOSE_UNSUPPORTED'
    });
  }
  if (marker === 'MISSING') {
    throw new DeployError('上一版本的生效 Compose 清单不存在，已停止部署', {
      code: 'OLD_COMPOSE_MISSING'
    });
  }
  if (marker === 'OUTSIDE') {
    throw new DeployError('上一版本 Compose 位于其他目录，本期不支持迁移部署目录', {
      code: 'OLD_COMPOSE_UNSUPPORTED'
    });
  }
  if (marker !== 'FOUND') {
    throw new DeployError('无法确认上一版本 Compose 清单路径，已停止部署', {
      code: 'OLD_COMPOSE_UNSUPPORTED'
    });
  }
  return path.posix.relative(deployDir, file);
}

export function uniqueImageRefs(config) {
  const result = [];
  for (const service of Object.values(config?.services || {})) {
    if (Array.isArray(service?.profiles) && service.profiles.length > 0) continue;
    const reference = String(service?.image || '').trim();
    if (reference && !result.includes(reference)) result.push(reference);
  }
  return result;
}

function activeServices(config) {
  return Object.entries(config?.services || {})
    .filter(([, service]) => !Array.isArray(service?.profiles) || service.profiles.length === 0)
    .map(([name]) => name);
}

export function validateProjectContainerScope(projectName, installDir, deployDir, raw) {
  const normalize = (value) => path.posix.normalize(String(value || '').replaceAll('\\', '/'));
  const inside = (base, target) => {
    const root = normalize(base).replace(/\/+$/, '');
    const value = normalize(target);
    return value === root || value.startsWith(`${root}/`);
  };
  let found = false;
  for (const line of String(raw || '').split('\n').map((item) => item.trim()).filter(Boolean)) {
    found = true;
    const workingDir = line.split('|')[0].trim();
    if (!workingDir) {
      throw new DeployError(`Compose 项目 ${projectName} 的容器缺少工作目录标签，无法确认部署边界`, {
        code: 'COMPOSE_PROJECT_SCOPE_UNKNOWN'
      });
    }
    if (!inside(deployDir, workingDir) || !inside(installDir, workingDir)) {
      throw new DeployError(
        `Compose 项目 ${projectName} 已被其他部署目录使用（${workingDir}），已阻止部署`,
        { code: 'COMPOSE_PROJECT_CONFLICT' }
      );
    }
  }
  return found;
}

function validateNewDeploymentIsolation(config, hasProjectContainers) {
  if (hasProjectContainers) return;
  for (const [name, service] of Object.entries(config?.services || {})) {
    if (!activeServices(config).includes(name)) continue;
    if (service.container_name) {
      throw new DeployError(`首次部署的服务 ${name} 使用固定容器名，无法确认是否会影响已有部署`, { code: 'FIRST_DEPLOY_CONFLICT' });
    }
    if ((service.ports || []).some((port) => !port.host_ip || port.host_ip === '127.0.0.1')) {
      throw new DeployError(`首次部署的服务 ${name} 使用可能冲突的宿主端口，请先隔离旧服务或改用专用端口`, { code: 'FIRST_DEPLOY_CONFLICT' });
    }
    if ((service.volumes || []).some((volume) => volume.type === 'volume' && !volume.source)) {
      throw new DeployError(`首次部署的服务 ${name} 包含匿名卷，部署失败后无法可靠清理`, { code: 'FIRST_DEPLOY_CONFLICT' });
    }
  }
}

export function buildImageOverrideDocument(config, imageCheck) {
  const imageIds = new Map(
    (imageCheck?.checks || [])
      .filter((image) => image?.present && image?.imageId)
      .map((image) => [String(image.ref || '').trim(), String(image.imageId).trim()])
  );
  const services = {};
  for (const [name, service] of Object.entries(config?.services || {})) {
    if (!activeServices(config).includes(name)) continue;
    const reference = String(service?.image || '').trim();
    const imageId = imageIds.get(reference);
    if (!imageId) {
      throw new DeployError(`服务 ${name} 的镜像 ${reference} 未通过完整镜像预检`, {
        code: 'SERVICE_IMAGE_NOT_READY'
      });
    }
    services[name] = { image: imageId, pull_policy: 'never' };
  }
  return Object.keys(services).length > 0
    ? JSON.stringify({ services })
    : '';
}

function volumeName(config, logical) {
  const volume = config?.volumes?.[logical];
  if (!volume) throw new DeployError(`Compose 未声明数据卷 ${logical}`, { code: 'DATA_VOLUME_MISSING' });
  if (volume.external) throw new DeployError(`数据卷 ${logical} 为 external，无法自动备份`, { code: 'DATA_VOLUME_EXTERNAL' });
  if (!volume.name) throw new DeployError(`无法解析数据卷 ${logical} 的实际名称`, { code: 'DATA_VOLUME_NAME_MISSING' });
  return String(volume.name);
}

function volumeMountedBy(config, serviceKey, logical) {
  const volumes = config?.services?.[serviceKey]?.volumes;
  return Array.isArray(volumes) && volumes.some((volume) =>
    volume?.type === 'volume' && volume?.source === logical);
}

function declaredVolumeNames(config) {
  const result = new Set();
  for (const service of Object.values(config?.services || {})) {
    for (const volume of Array.isArray(service?.volumes) ? service.volumes : []) {
      if (volume?.type === 'volume') result.add(String(volume.source || ''));
    }
  }
  return result;
}

function mountPathContains(root, target) {
  const normalize = (value) => {
    let text = String(value || '').replaceAll('\\', '/').replace(/^\/\/\?\//, '');
    const windows = /^[A-Za-z]:/.test(text);
    text = path.posix.normalize(text).replace(/^\/+|\/+$/g, '');
    return windows ? text.toLowerCase() : text;
  };
  const base = normalize(root);
  const value = normalize(target);
  return Boolean(base) && (base === value || value.startsWith(`${base}/`));
}

export function validateMountContract(config, { volumes, dataServices, preserve, managed }) {
  if (!config?.services || typeof config.services !== 'object') {
    throw new DeployError('Compose 缺少服务清单', { code: 'MOUNTS_INVALID' });
  }
  for (const [name, service] of Object.entries(config.services)) {
    if (service.volumes_from?.length) {
      throw new DeployError(`服务 ${name} 使用继承数据卷，无法确认数据保护范围`, { code: 'MOUNTS_INVALID' });
    }
    for (const mount of service.volumes || []) {
      const source = String(mount.source || '');
      if (mount.type === 'volume') {
        if (!source || !volumes.includes(source)) {
          throw new DeployError(`服务 ${name} 使用匿名卷或未声明数据卷 ${source}，无法安全恢复`, { code: 'MOUNTS_INVALID' });
        }
        volumeName(config, source);
        if (Object.keys(config.volumes[source]?.driver_opts || {}).length) {
          throw new DeployError(`数据卷 ${source} 使用外部存储选项，本期无法自动恢复`, { code: 'MOUNTS_INVALID' });
        }
      } else if (mount.type === 'bind') {
        const preserved = preserve.some((root) => mountPathContains(root, source));
        const snapshotted = managed.some((root) => mountPathContains(root, source));
        if ((mount.read_only && (preserved || snapshotted))
          || (preserved && !snapshotted && !dataServices.includes(name))) continue;
        throw new DeployError(`服务 ${name} 的宿主挂载 ${source} 未纳入可恢复数据范围，请改用声明的数据卷或只读受管文件`, { code: 'MOUNTS_INVALID' });
      } else if (mount.type !== 'tmpfs') {
        throw new DeployError(`服务 ${name} 使用不支持的挂载类型 ${mount.type || ''}`, { code: 'MOUNTS_INVALID' });
      }
    }
  }
}

function validateDeploymentMounts(bundle, installDir, managed, config) {
  const data = bundle.descriptor.runtime.data;
  return validateMountContract(config, {
    volumes: data.volumes.map((volume) => volume.name),
    dataServices: bundle.descriptor.runtime.services.filter((service) => service.role === 'data').map((service) => service.serviceKey),
    preserve: (data.paths || []).map((item) => remoteJoin(installDir, item.path)),
    managed: managed.map((item) => remoteJoin(installDir, item))
  });
}

function managedPathsFor(bundle, installDir, deployDir, record, oldCompose, newCompose) {
  const newManaged = new Set();
  const result = new Set();
  const selected = bundle.selectComponents(bundle.descriptor.components.map((component) => component.key));
  for (const artifact of bundle.resolveArtifacts(selected, installDir, deployDir)) {
    const relative = relativeInstallPath(installDir, artifact.target);
    newManaged.add(relative);
    result.add(relative);
  }
  const envRelative = remoteJoin(bundle.deploySourceRel(), '.env');
  newManaged.add(envRelative);
  result.add(envRelative);
  newManaged.add('.deploy-desktop/deployed.json');
  result.add('.deploy-desktop/deployed.json');
  if (newCompose) {
    newManaged.add(newCompose);
    result.add(newCompose);
  }
  const retired = new Set();
  for (const artifact of record?.artifacts || []) {
    const relative = artifact.root === 'install'
      ? artifact.path
      : remoteJoin(bundle.deploySourceRel(), artifact.path);
    if (relative) {
      validateManagedRelative(relative, { allowRecord: false });
      result.add(relative);
      if (!newManaged.has(relative)) retired.add(relative);
    }
  }
  if (oldCompose) {
    result.add(oldCompose);
    if (!newManaged.has(oldCompose)) retired.add(oldCompose);
  }
  for (const relative of [...result, ...retired]) validateManagedRelative(relative);
  return { managed: [...result], retired: minimalRetired([...retired]) };
}

function minimalRetired(paths) {
  const result = [];
  for (const path of paths.sort()) {
    if (result.some((parent) => path === parent || path.startsWith(`${parent}/`))) continue;
    result.push(path);
  }
  return result;
}

function assertPreservedPathsUntouched(paths, bundle) {
  const preserved = bundle.descriptor.runtime.data?.paths || [];
  for (const item of preserved) {
    const preserve = String(item.path || '').trim();
    if (!preserve) continue;
    for (const path of paths) {
      if (path === preserve
          || path.startsWith(`${preserve}/`)
          || preserve.startsWith(`${path}/`)) {
        throw new DeployError(
          `部署受管路径 ${path} 与保留数据目录 ${preserve} 重叠，已阻止部署以保护现有数据`,
          { code: 'PRESERVED_PATH_CONFLICT' }
        );
      }
    }
  }
}

async function prepareRemoteTransaction(hostConfig, txn, incoming) {
  const result = await runRemote(hostConfig, `set -e
umask 077
mkdir -p ${shellQuote(incoming)}/tree
`, { timeoutMs: 60000 });
  return result;
}

async function writeRemoteText(hostConfig, remotePath, content, privateFile = false) {
  const temp = path.join(process.env.TMPDIR || '/tmp', `deploy-desktop-${process.pid}-${Date.now()}.tmp`);
  await fs.writeFile(temp, content, { mode: privateFile ? 0o600 : 0o644 });
  try {
    await uploadFile(privateFile ? { ...hostConfig, strictHostKeyChecking: 'yes' } : hostConfig, temp, remotePath);
  } finally {
    await fs.rm(temp, { force: true });
  }
}

function composeConfigScript(temp, deployDir, oldCompose, envUploaded, installDir) {
  return `set -eo pipefail
${deploymentBaselineScript}
TMP=${shellQuote(temp)}
DEPLOY=${shellQuote(deployDir)}
docker compose version >/dev/null 2>&1 || { echo '目标机 Docker Compose 不支持完整版本部署' >&2; exit 3; }
ENV_ARGS=()
[ -f "$DEPLOY/.env" ] && ENV_ARGS+=(--env-file "$DEPLOY/.env")
${envUploaded ? 'ENV_ARGS+=(--env-file "$TMP/env")' : ''}
[ -f "$DEPLOY/.env.local" ] && ENV_ARGS+=(--env-file "$DEPLOY/.env.local")
docker compose --project-directory "$DEPLOY" \${ENV_ARGS[@]+"\${ENV_ARGS[@]}"} -f "$TMP/compose" config --format json > "$TMP/incoming.json"
printf 'INCOMING_CONFIG_B64='
base64 < "$TMP/incoming.json" | tr -d '\\n'
printf '\\n'
OLD=${shellQuote(oldCompose || '')}
printf 'BASELINE=%s\\n' "$(deployment_baseline ${shellQuote(installDir)} "$DEPLOY" "$OLD")"
if [ -n "$OLD" ] && [ -f "$DEPLOY/$OLD" ]; then
  ENV_ARGS=()
  [ -f "$DEPLOY/.env" ] && ENV_ARGS+=(--env-file "$DEPLOY/.env")
  [ -f "$DEPLOY/.env.local" ] && ENV_ARGS+=(--env-file "$DEPLOY/.env.local")
  docker compose --project-directory "$DEPLOY" \${ENV_ARGS[@]+"\${ENV_ARGS[@]}"} -f "$DEPLOY/$OLD" config --format json > "$TMP/current.json"
  printf 'HAS_DEPLOYMENT=yes\\n'
  printf 'CURRENT_CONFIG_B64='
  base64 < "$TMP/current.json" | tr -d '\\n'
  printf '\\n'
else
  printf 'HAS_DEPLOYMENT=no\\n'
fi
`;
}

function parseMarkedBase64(raw, key) {
  const line = String(raw || '').split('\n').find((item) => item.startsWith(`${key}=`));
  if (!line) throw new DeployError(`目标机预检结果缺少 ${key}`, { code: 'PREFLIGHT_INCOMPLETE' });
  try {
    return JSON.parse(decodeBase64(line.slice(key.length + 1)).toString('utf8'));
  } catch {
    throw new DeployError('目标机 Compose 配置无法解析', { code: 'PREFLIGHT_INVALID' });
  }
}

async function remoteImageBindings(hostConfig, installDir) {
  const file = `${installDir}/.deploy-desktop/image-bindings.json`;
  const result = await runRemote(hostConfig, `set -e
[ ! -L ${shellQuote(file)} ] || exit 2
if [ -e ${shellQuote(file)} ]; then cat ${shellQuote(file)}; fi
`, { timeoutMs: 60000 });
  if (!result.stdout.trim()) return new Map();
  try {
    const raw = JSON.parse(result.stdout);
    if (raw.schemaVersion !== 1 || !Array.isArray(raw.images)) throw new Error();
    const bindings = new Map();
    for (const image of raw.images) {
      if (!/^sha256:[a-f0-9]{64}$/i.test(image.imageId) || !image.reference?.includes('@sha256:')) {
        throw new Error();
      }
      bindings.set(image.reference, image.imageId);
    }
    return bindings;
  } catch {
    throw new DeployError('目标机的离线镜像记录无效，请重新核对镜像', { code: 'IMAGE_BINDINGS_INVALID' });
  }
}

function volumeCheckScript(
  plans,
  oldImages,
  bindings = new Map(),
  projectName = '',
  recordedOldImages = new Map()
) {
  const lines = [
    'set -e',
    'docker info >/dev/null 2>&1 || { echo "Docker 未运行或当前用户无权访问" >&2; exit 2; }',
    'command -v tar >/dev/null 2>&1 || { echo "目标机缺少 tar" >&2; exit 2; }',
    'command -v gzip >/dev/null 2>&1 || { echo "目标机缺少 gzip" >&2; exit 2; }',
    'command -v sha256sum >/dev/null 2>&1 || { echo "目标机缺少 sha256sum" >&2; exit 2; }',
    'command -v timeout >/dev/null 2>&1 || { echo "目标机缺少 timeout，无法限制备份与恢复耗时" >&2; exit 2; }',
    'command -v flock >/dev/null 2>&1 || { echo "目标机缺少 flock，无法获取部署锁" >&2; exit 2; }'
  ];
  oldImages.forEach((reference, index) => {
    const target = recordedOldImages.get(reference) || bindings.get(reference) || reference;
    lines.push(`OLD_PROBE=$(docker image inspect --format '{{.Id}}|{{.Os}}/{{.Architecture}}' -- ${shellQuote(target)} 2>/dev/null) || { echo ${shellQuote(`缺少上一版本镜像 ${reference}`)} >&2; exit 5; }
printf 'OLD_IMAGE|${index}|%s\\n' "$OLD_PROBE"`);
  });
  plans.forEach((volume, index) => {
    lines.push(`if docker volume inspect -- ${shellQuote(volume.actual)} >/dev/null 2>&1; then
  exists=yes
  owner=$(docker volume inspect --format '{{index .Labels "com.docker.compose.project"}}|{{index .Labels "com.docker.compose.volume"}}' -- ${shellQuote(volume.actual)})
  [ "$owner" = ${shellQuote(`${projectName}|${volume.logical}`)} ] || { echo '数据卷 Compose 归属标签不一致，已阻止部署' >&2; exit 6; }
  bytes=$(docker run --rm --network none -v ${shellQuote(`${volume.actual}:/data:ro`)} --entrypoint /bin/sh ${shellQuote(volume.image)} -c "du -sk /data 2>/dev/null | awk '{print \\$1}'" 2>/dev/null | tr -d '\\r')
  bytes=$(( \${bytes:-0} * 1024 ))
else
  exists=no
  bytes=0
fi
if ! docker run --rm --network none --entrypoint /bin/sh ${shellQuote(volume.image)} -c 'command -v tar >/dev/null && command -v gzip >/dev/null' >/dev/null 2>&1; then
  echo ${shellQuote(`备份镜像 ${volume.image} 缺少 tar 或 gzip`)} >&2
  exit 6
fi
printf 'VOLUME|${index}|%s|%s\\n' "$exists" "$bytes"`);
  });
  return lines.join('\n');
}

async function preflightRemote({ hostConfig, bundle, profile, installDir }) {
  const deployDir = remoteJoin(installDir, bundle.deploySourceRel());
  const record = await remoteReadRecord(hostConfig, installDir);
  if (record && record.product !== bundle.product()) {
    throw new DeployError('目标目录的旧部署记录与当前产品不匹配，已停止部署', { code: 'OLD_PRODUCT_MISMATCH' });
  }
  const oldCompose = await findOldCompose(hostConfig, installDir, deployDir, record);
  const hadDeployment = Boolean(oldCompose);
  const newComposeArtifact = bundle.artifactForSource(bundle.effectiveCompose());
  if (!newComposeArtifact) throw new DeployError('交付包未声明 Compose 清单', { code: 'BUNDLE_INVALID' });
  const env = await readProjectEnv(profile.projectId);
  const temp = `/tmp/deploy-desktop-preflight-${Date.now()}-${process.pid}`;
  try {
    await runRemote(hostConfig, `mkdir -p ${shellQuote(temp)}`, { timeoutMs: 30000 });
    const composeSource = path.join(bundle.root, bundle.effectiveCompose());
    await uploadFile(hostConfig, composeSource, `${temp}/compose`);
    if (env.trim()) await writeRemoteText(hostConfig, `${temp}/env`, env, true);
    const configResult = await runRemote(hostConfig, composeConfigScript(temp, deployDir, oldCompose, Boolean(env.trim()), installDir), {
      timeoutMs: 300000
    });
    const incoming = parseMarkedBase64(configResult.stdout, 'INCOMING_CONFIG_B64');
    const hasDeployment = configResult.stdout.split('\n').some((line) => line.trim() === 'HAS_DEPLOYMENT=yes');
    const current = hasDeployment ? parseMarkedBase64(configResult.stdout, 'CURRENT_CONFIG_B64') : null;
    const projectName = String(incoming.name || '').trim();
    if (!projectName) throw new DeployError('新版本 Compose 清单缺少项目名称', { code: 'COMPOSE_PROJECT_MISSING' });
    const projectContainers = validateProjectContainerScope(
      projectName,
      installDir,
      deployDir,
      (await runRemote(hostConfig,
        `docker ps -a --filter ${shellQuote(`label=com.docker.compose.project=${projectName}`)} --format '{{.Label "com.docker.compose.project.working_dir"}}|{{.Label "com.docker.compose.project.config_files"}}'`,
        { timeoutMs: 60000 })).stdout
    );
    if (!hasDeployment && projectContainers) {
      throw new DeployError(
        `Compose 项目 ${projectName} 已有容器，但当前目录没有可恢复的部署记录，已阻止部署`,
        { code: 'COMPOSE_PROJECT_UNRECORDED' }
      );
    }
    if (!hasDeployment) validateNewDeploymentIsolation(incoming, false);
    if (current && String(current.name || '') !== projectName) {
      throw new DeployError(`新旧版本 Compose 项目名称不同（${current.name} -> ${projectName}），已阻止部署`, {
        code: 'COMPOSE_PROJECT_CHANGED'
      });
    }
    const services = activeServices(incoming);
    if (services.some((key) => !bundle.descriptor.runtime.services.some((service) => service.serviceKey === key))) {
      throw new DeployError('Compose 包含未声明角色的服务，请重新构建制品', { code: 'SERVICE_ROLE_MISSING' });
    }
    const missingServices = bundle.descriptor.runtime.services
      .map((service) => service.serviceKey)
      .filter((key) => !services.includes(key));
    if (missingServices.length > 0) {
      throw new DeployError(`交付包声明的服务未出现在 Compose 配置：${missingServices.join('、')}`, {
        code: 'SERVICE_DECLARATION_MISMATCH'
      });
    }
    const volumePlans = [];
    const data = bundle.ensureFullDeployMetadata();
    for (const volume of data.volumes) {
      if (!volumeMountedBy(incoming, volume.service, volume.name)) {
        throw new DeployError(`数据卷 ${volume.name} 未挂载到声明的服务 ${volume.service}`, { code: 'DATA_VOLUME_NOT_MOUNTED' });
      }
      const actual = volumeName(incoming, volume.name);
      if (current && volumeName(current, volume.name) !== actual) {
        throw new DeployError(`新旧版本数据卷 ${volume.name} 的实际名称不一致`, { code: 'DATA_VOLUME_CHANGED' });
      }
      const backupImage = bundle.descriptor.images.find((image) => image.name === volume.image);
      if (!backupImage) {
        throw new DeployError(`数据卷 ${volume.name} 的备份镜像未声明`, { code: 'BACKUP_IMAGE_MISSING' });
      }
      volumePlans.push({
        logical: volume.name,
        service: volume.service,
        actual,
        image: backupImage.imageRef,
        exists: false,
        bytes: 0,
        required: Boolean(volume.required)
      });
    }
    const declared = declaredVolumeNames(incoming);
    const undeclared = [...declared].filter((name) => !volumePlans.some((volume) => volume.logical === name));
    if (undeclared.length > 0) {
      throw new DeployError(`服务使用了未声明的数据卷 ${undeclared.join('、')}，无法保证失败后恢复数据`, {
        code: 'DATA_VOLUME_UNDECLARED'
      });
    }
    const imageCheck = await verifyImagesRemote(hostConfig, bundle);
    if (imageCheck.problems.length > 0) {
      throw new DeployError(`目标机镜像预检未通过：${imageCheck.problems.join('；')}`, {
        code: 'IMAGE_PRECHECK_FAILED',
        details: imageCheck
      });
    }
    buildImageOverrideDocument(incoming, imageCheck);
    const oldImages = current ? uniqueImageRefs(current) : [];
    const bindings = await remoteImageBindings(hostConfig, installDir);
    for (const volume of volumePlans) {
      const image = imageCheck.checks.find((image) => image.ref === volume.image && image.present);
      if (!image?.imageId) {
        throw new DeployError(`数据卷 ${volume.logical} 的备份镜像尚未准备完成`, {
          code: 'BACKUP_IMAGE_MISSING'
        });
      }
      volume.image = image.imageId;
    }
    const recordedOldImages = new Map();
    for (const image of record?.images || []) {
      if (!/^(?:sha256:)?[a-f0-9]{64}$/i.test(String(image.imageId || ''))) {
        throw new DeployError(`上一版本部署记录中的镜像 ${image.reference} 标识无效`, {
          code: 'OLD_IMAGE_RECORD_INVALID'
        });
      }
      recordedOldImages.set(image.reference, image.imageId);
    }
    const checkScript = volumeCheckScript(
      volumePlans,
      oldImages,
      bindings,
      projectName,
      recordedOldImages
    );
    const spaceResult = await runRemote(hostConfig, `set -e
${checkScript}
SPACE_PATH=${shellQuote(installDir)}
[ -e "$SPACE_PATH" ] || SPACE_PATH=$(dirname "$SPACE_PATH")
printf 'FREE_KB=%s\\n' "$(df -Pk "$SPACE_PATH" | awk 'NR==2{print $4}')"
`, { timeoutMs: 600000 });
    for (const line of spaceResult.stdout.split('\n')) {
      if (!line.startsWith('VOLUME|')) continue;
      const [, index, exists, bytes] = line.split('|');
      const volume = volumePlans[Number(index)];
      if (!volume) continue;
      volume.exists = exists === 'yes';
      volume.bytes = Number(bytes) || 0;
    }
    const oldImageChecks = [];
    for (const line of spaceResult.stdout.split('\n')) {
      if (!line.startsWith('OLD_IMAGE|')) continue;
      const [, rawIndex, imageId, osArch] = line.split('|', 4);
      const index = Number(rawIndex);
      const reference = oldImages[index];
      if (!reference || !imageId) continue;
      oldImageChecks.push({ ref: reference, present: true, imageId, osArch });
    }
    if (oldImageChecks.length !== oldImages.length) {
      throw new DeployError('上一版本镜像状态核对不完整', {
        code: 'OLD_IMAGE_PRECHECK_INCOMPLETE'
      });
    }
    const recordedChecks = [];
    for (const [index, image] of oldImageChecks.entries()) {
      const recorded = (record?.images || []).find((item) => item.reference === image.ref);
      if (!recorded) continue;
      if (!/^(?:sha256:)?[a-f0-9]{64}$/i.test(String(recorded.imageId || ''))) {
        throw new DeployError(`上一版本部署记录中的镜像 ${image.ref} 标识无效`, {
          code: 'OLD_IMAGE_RECORD_INVALID'
        });
      }
      recordedChecks.push({ index, image, recorded });
    }
    if (recordedChecks.length > 0) {
      const script = recordedChecks.map(({ index, recorded }) =>
        `value=$(docker image inspect --format '{{.Id}}|{{.Os}}/{{.Architecture}}' -- ${shellQuote(recorded.imageId)} 2>/dev/null) || { echo '上一版本镜像已不存在' >&2; exit 5; }
printf 'RECORDED_OLD|${index}|%s\\n' "$value"`
      ).join('\n');
      const pinned = await runRemote(hostConfig, `set -e\n${script}`, { timeoutMs: 120000 });
      const found = new Set();
      for (const line of pinned.stdout.split('\n').filter((item) => item.startsWith('RECORDED_OLD|'))) {
        const [, rawIndex, imageId, osArch] = line.split('|', 4);
        const index = Number(rawIndex);
        if (!oldImageChecks[index] || !imageId) continue;
        oldImageChecks[index].imageId = imageId;
        oldImageChecks[index].osArch = osArch;
        found.add(index);
      }
      if (found.size !== recordedChecks.length) {
        throw new DeployError('上一版本部署记录中的镜像状态核对不完整', {
          code: 'OLD_IMAGE_RECORD_INCOMPLETE'
        });
      }
    }
    for (const volume of volumePlans) {
      if (!hadDeployment && volume.exists) {
        throw new DeployError(`首次部署的目标已存在数据卷 ${volume.actual}，为避免误删已有数据已停止部署`, {
          code: 'DATA_VOLUME_ALREADY_EXISTS'
        });
      }
      if (hadDeployment && volume.required && !volume.exists) {
        throw new DeployError(`现有部署缺少必需数据卷 ${volume.actual}，无法建立可恢复快照`, {
          code: 'DATA_VOLUME_MISSING'
        });
      }
    }
    const freeLine = spaceResult.stdout.split('\n').find((line) => line.startsWith('FREE_KB='));
    const freeKb = Number(freeLine?.slice('FREE_KB='.length)) || 0;
    let required = volumePlans.reduce((total, volume) => total + volume.bytes, 0)
      + 2 * bundle.descriptor.files.reduce((total, file) => total + Number(file.size || 0), 0)
      + 512 * 1024 * 1024;
    if (freeKb * 1024 < required) {
      throw new DeployError(
        `目标机空间不足：需要约 ${(required / 1024 / 1024 / 1024).toFixed(1)} GB，可用 ${(freeKb / 1024 / 1024).toFixed(1)} GB`,
        { code: 'INSUFFICIENT_DISK' }
      );
    }
    const newComposeTarget = newComposeArtifact.root === 'install'
      ? remoteJoin(installDir, newComposeArtifact.path)
      : remoteJoin(deployDir, newComposeArtifact.path);
    const newCompose = path.posix.relative(deployDir, newComposeTarget) || newComposeArtifact.path;
    const newComposeManaged = relativeInstallPath(installDir, newComposeTarget);
    const oldComposeManaged = oldCompose
      ? path.posix.normalize(remoteJoin(bundle.deploySourceRel(), oldCompose))
      : '';
    const managed = managedPathsFor(
      bundle,
      installDir,
      deployDir,
      record,
      oldComposeManaged,
      newComposeManaged
    );
    validateDeploymentMounts(bundle, installDir, managed.managed, incoming);
    if (current) validateDeploymentMounts(bundle, installDir, managed.managed, current);
    assertPreservedPathsUntouched(managed.managed, bundle);
    assertPreservedPathsUntouched(managed.retired, bundle);
    const fileSize = await runRemote(hostConfig, `set -eo pipefail
${deploymentBaselineScript}
deployment_managed_bytes ${[installDir, ...managed.managed].map(shellQuote).join(' ')}
`, { timeoutMs: 300000 });
    const snapshotBytes = Number(fileSize.stdout.trim());
    if (!Number.isSafeInteger(snapshotBytes) || snapshotBytes < 0) {
      throw new DeployError('无法估算目标文件快照空间', { code: 'PREFLIGHT_INCOMPLETE' });
    }
    required += snapshotBytes;
    if (freeKb * 1024 < required) {
      throw new DeployError(`目标机空间不足：含文件快照与暂存内容需要约 ${(required / 1024 ** 3).toFixed(1)} GB`, { code: 'INSUFFICIENT_DISK' });
    }
    return {
      record,
      hadDeployment,
      previousVersion: String(record?.version || ''),
      oldCompose,
      newCompose,
      projectName,
      services,
      oldServices: current ? activeServices(current) : [],
      volumes: volumePlans,
      imageCheck,
      incoming,
      current,
      oldImages,
      oldImageChecks,
      managedPaths: managed.managed,
      retiredPaths: managed.retired,
      requiredBackupBytes: required,
      baseline: configResult.stdout.split('\n').find((line) => line.startsWith('BASELINE='))?.slice(9) || '',
      env
    };
  } finally {
    await runRemote(hostConfig, `rm -rf -- ${shellQuote(temp)}`, { timeoutMs: 30000 }).catch(() => {});
  }
}

function deployRecord({ bundle, profile, installDir, deployDir, imageCheck, volumes, transactionId, env }) {
  const composeFiles = [bundle.descriptor.runtime.compose, bundle.effectiveCompose()]
    .filter(Boolean)
    .flatMap((source) => {
      const artifact = bundle.artifactForSource(source);
      if (!artifact) return [];
      const base = artifact.root === 'install' ? installDir : deployDir;
      const file = bundle.descriptor.files.find((item) => item.path === source);
      return [{
        path: remoteJoin(base, artifact.path),
        sha256: String(file?.sha256 || '')
      }];
    });
  return {
    schemaVersion: 2,
    projectId: String(profile.projectId || ''),
    product: bundle.product(),
    version: bundle.version(),
    installedAt: new Date().toISOString(),
    artifacts: bundle.descriptor.artifacts.map((artifact) => ({
      key: artifact.key,
      root: artifact.root,
      path: artifact.path,
      kind: artifact.kind
    })),
    images: (imageCheck.checks || [])
      .filter((image) => image.present)
      .map((image) => ({
        reference: image.ref,
        imageId: String(image.imageId || ''),
        platform: String(image.osArch || '')
      })),
    composeFiles,
    effectiveCompose: relativeInstallPath(
      installDir,
      remoteJoin(deployDir, bundle.artifactForSource(bundle.effectiveCompose()).path)
    ),
    envSha256: env.trim() ? crypto.createHash('sha256').update(env).digest('hex') : '',
    dataVolumes: volumes.map((volume) => volume.logical),
    transactionId
  };
}

function transactionJournal({ id, bundle, installDir, deployDir, plan }) {
  const now = new Date().toISOString();
  return {
    schemaVersion: 1,
    id,
    product: bundle.product(),
    version: bundle.version(),
    previousVersion: plan.previousVersion,
    installDir,
    deployDir,
    phase: 'preparing',
    state: 'pending',
    startedAt: now,
    updatedAt: now,
    snapshotDir: `${installDir}/.deploy-desktop/transactions/${id}/snapshot`,
    hadDeployment: plan.hadDeployment,
    deploymentStarted: false,
    projectName: plan.projectName,
    previousCompose: plan.oldCompose,
    newCompose: plan.newCompose,
    files: [],
    volumes: [],
    oldImages: plan.oldImageChecks.map((image) => ({
      reference: image.ref,
      imageId: image.imageId,
      platform: image.osArch
    })),
    services: plan.services,
    previousServices: plan.oldServices,
    frontendService: bundle.descriptor.runtime.services.find((service) => service.key.toLowerCase() === 'frontend')?.serviceKey || '',
    frontendPort: bundle.descriptor.runtime.services.find((service) => service.key.toLowerCase() === 'frontend')?.port || null,
    error: null,
    recoveryError: null
  };
}

function workerArgs({
  installDir,
  deployDir,
  txn,
  plan,
  overrideFile = '',
  oldOverrideFile = ''
}) {
  return [
    installDir,
    deployDir,
    txn,
    plan.hadDeployment ? 'yes' : 'no',
    plan.oldCompose,
    plan.newCompose,
    bundleFrontendService(plan),
    String(plan.frontendPort || ''),
    plan.services.join(' '),
    plan.oldServices.join(' '),
    overrideFile,
    oldOverrideFile
  ];
}

function projectLockPath(projectName) {
  const digest = crypto.createHash('sha256').update(String(projectName || '').trim()).digest('hex');
  return `/tmp/deploy-desktop-project-locks/${digest}.lock`;
}

function bundleFrontendService(plan) {
  return String(plan.frontendService || '');
}

export function publishedFrontendPort(config, service, preferred = null) {
  const ports = (config?.services?.[service]?.ports || [])
    .filter((item) => (item.protocol || 'tcp') === 'tcp')
    .map((item) => Number(item.published))
    .filter((port) => Number.isInteger(port) && port > 0 && port <= 65535);
  return ports.includes(preferred) ? preferred : ports.length === 1 ? ports[0] : null;
}

async function launchWorker(hostConfig, txn, mode, args) {
  await runRemote(hostConfig, versionDeployLaunchScript(txn, mode, args), { timeoutMs: 60000 });
}

async function pollTransaction(hostConfig, installDir, timeoutMs, onProgress = () => {}, initial = null) {
  const deadline = Date.now() + timeoutMs;
  let lastPhase = '';
  let lastPercent = 50;
  while (Date.now() < deadline) {
    const transaction = await inspectRemoteTransaction(hostConfig, installDir);
    if (transaction) {
      const percent = {
        preparing: 55,
        snapshotting_files: 60,
        snapshotting_data: 67,
        applying: 78,
        starting: 88,
        recovering: 92,
        committed: 100,
        recovered: 100,
        recovery_failed: 100
      }[transaction.phase] || lastPercent;
      if (transaction.phase !== lastPhase || percent > lastPercent) {
        lastPhase = transaction.phase;
        lastPercent = Math.max(lastPercent, percent);
        onProgress(lastPercent, phaseLabel(transaction.phase));
      }
      if (isTerminalTransaction(transaction, initial)) return transaction;
    }
    await new Promise((resolve) => setTimeout(resolve, 2000));
  }
  throw new DeployError('等待目标机部署事务超时。任务可能仍在运行，请重新连接后检查事务状态', {
    code: 'TRANSACTION_TIMEOUT'
  });
}

function phaseLabel(phase) {
  return {
    preparing: '预检部署事务',
    snapshotting_files: '保存部署文件',
    snapshotting_data: '备份数据卷',
    applying: '写入完整版本',
    starting: '启动并验证',
    recovering: '恢复上一版本',
    committed: '部署完成',
    recovered: '已恢复上一版本',
    recovery_failed: '自动恢复失败'
  }[phase] || '部署事务执行中';
}

export function isTerminalTransaction(transaction, initial = null) {
  if (!TERMINAL_STATES.has(transaction?.state)) return false;
  if (!initial) return true;
  return transaction.state !== initial.state || transaction.updatedAt !== initial.updatedAt;
}

async function readWorkerTail(hostConfig, txn) {
  const result = await runRemote(hostConfig, `if [ -f ${shellQuote(`${txn}/worker.log`)} ]; then tail -n 160 ${shellQuote(`${txn}/worker.log`)}; fi`, {
    timeoutMs: 60000
  }).catch(() => ({ stdout: '' }));
  return redactDiagnostics(result.stdout);
}

function resultFromTransaction(transaction, { success, recovered, recoveryFailed, error, output, durationSecs }) {
  return {
    success,
    recovered,
    recoveryFailed,
    stage: transaction.phase,
    version: transaction.version,
    previousVersion: transaction.previousVersion,
    transactionId: transaction.id,
    snapshotDir: transaction.snapshotDir,
    error: error || null,
    output,
    durationSecs,
    status: transaction
  };
}

export async function deployVersionRemote(profile, hostConfig, options = {}) {
  const started = Date.now();
  const progress = options.onProgress || (() => {});
  const installDir = trimSlashes(validateInstallDir(options.installDir || hostConfig.installDir || profile.installDir));
  const bundleDir = path.resolve(expandHome(options.bundleDir || profile.bundleDir));
  progress(3, '校验交付包');
    const bundle = await Bundle.load(bundleDir, { appVersion: await appVersion(), verify: true });
  if (bundle.product() !== profile.product) {
    throw new DeployError(`交付包属于「${bundle.product()}」，当前项目使用「${profile.product}」`, { code: 'PRODUCT_MISMATCH' });
  }
  bundle.ensureFullDeployMetadata();
  const existing = await inspectRemoteTransaction(hostConfig, installDir);
  assertNoBlockingTransaction(existing);
  progress(8, '预检目标机');
  const plan = await preflightRemote({ hostConfig, bundle, profile, installDir });
  const frontend = bundle.descriptor.runtime.services
    .find((service) => service.key.toLowerCase() === 'frontend');
  plan.frontendService = frontend?.serviceKey || '';
  plan.frontendPort = frontend?.port || null;
  plan.oldFrontendPort = publishedFrontendPort(plan.current, plan.frontendService, plan.frontendPort);
  if (plan.current?.services?.[plan.frontendService] && !plan.oldFrontendPort) {
    throw new DeployError('无法确定上一版本网页的 HTTP 端口，请确认旧版本的网页端口声明后再部署', { code: 'FRONTEND_PORT_AMBIGUOUS' });
  }
  const dryRun = Boolean(options.dryRun);
  if (dryRun) {
    progress(100, '预检通过');
    return {
      success: true,
      recovered: false,
      recoveryFailed: false,
      stage: 'preparing',
      version: bundle.version(),
      previousVersion: plan.previousVersion,
      transactionId: '',
      snapshotDir: '',
      error: null,
      output: `预检通过：目标 ${installDir}；版本 ${bundle.version()}；${plan.services.length} 个服务；数据卷：${plan.volumes.map((volume) => volume.logical).join('、') || '无'}；预计备份空间约 ${(plan.requiredBackupBytes / 1024 / 1024 / 1024).toFixed(1)} GB。未上传部署文件，也未停止服务。`,
      durationSecs: Math.round((Date.now() - started) / 1000),
      status: null
    };
  }
  const env = plan.env;
  const id = `deploy-${Date.now()}-${process.pid}-${crypto.randomUUID()}`;
  const txn = `${installDir}/.deploy-desktop/transactions/${id}`;
  const incoming = `${txn}/incoming`;
  const deployDir = remoteJoin(installDir, bundle.deploySourceRel());
  progress(14, '上传完整版本');
  await prepareRemoteTransaction(hostConfig, txn, incoming);
  let overrideFile = '';
  let oldOverrideFile = '';
  try {
    const uploadPaths = [];
    const selected = bundle.selectComponents(bundle.descriptor.components.map((component) => component.key));
    const artifacts = bundle.resolveArtifacts(selected, installDir, deployDir);
    for (const [index, artifact] of artifacts.entries()) {
      const relative = relativeInstallPath(installDir, artifact.target);
      uploadPaths.push(relative);
      progress(16 + Math.round(((index + 1) / Math.max(artifacts.length, 1)) * 34), `上传${artifact.label || artifact.key}`);
      if (artifact.kind === 'file') {
        await uploadFile(hostConfig, artifact.source, `${incoming}/tree/${relative}`);
      } else {
        await uploadDir(hostConfig, artifact.source, `${incoming}/tree/${relative}`);
      }
    }
    if (env.trim()) {
      await writeRemoteText(hostConfig, `${incoming}/env`, env, true);
      await writeRemoteText(hostConfig, `${incoming}/env-present`, 'yes\n');
    }
    await writeRemoteText(hostConfig, `${incoming}/upload-paths.txt`, `${uploadPaths.join('\n')}\n`);
    await writeRemoteText(hostConfig, `${incoming}/managed-paths.txt`, `${plan.managedPaths.join('\n')}\n`);
    await writeRemoteText(hostConfig, `${incoming}/remove-paths.txt`, `${plan.retiredPaths.join('\n')}\n`);
    await writeRemoteText(
      hostConfig,
      `${incoming}/volume-specs.tsv`,
      `${plan.volumes.map((volume) => `${volume.logical}\t${volume.actual}\t${volume.image}\t${volume.exists ? 'yes' : 'no'}`).join('\n')}\n`
    );
    await writeRemoteText(hostConfig, `${incoming}/deployed.json`, `${JSON.stringify(deployRecord({
      bundle,
      profile,
      installDir,
      deployDir,
      imageCheck: plan.imageCheck,
      volumes: plan.volumes,
      transactionId: id,
      env
    }), null, 2)}\n`, true);
    const overrideDocument = buildImageOverrideDocument(plan.incoming, plan.imageCheck);
    overrideFile = overrideDocument ? `${txn}/image-override.compose.json` : '';
    if (overrideFile) {
      await writeRemoteText(hostConfig, overrideFile, `${overrideDocument}\n`, true);
    }
    const oldOverrideDocument = buildImageOverrideDocument(plan.current, {
      checks: plan.oldImageChecks
    });
    oldOverrideFile = `${txn}/old-image-override.compose.json`;
    await writeRemoteText(
      hostConfig,
      oldOverrideFile,
      `${oldOverrideDocument || '{"services":{}}'}\n`,
      true
    );
    await writeRemoteText(hostConfig, `${txn}/journal.json`, `${JSON.stringify(transactionJournal({
      id,
      bundle,
      installDir,
      deployDir,
      plan
    }), null, 2)}\n`);
    await writeRemoteText(hostConfig, `${txn}/worker.sh`, versionDeployScript);
    await writeRemoteText(hostConfig, `${txn}/project-name`, plan.projectName);
    await writeRemoteText(hostConfig, `${txn}/project-lock`, projectLockPath(plan.projectName));
    await writeRemoteText(hostConfig, `${txn}/baseline.sh`, deploymentBaselineScript);
    if (!/^[a-f0-9]{64}$/.test(plan.baseline)) {
      throw new DeployError('目标机预检缺少部署基线摘要', { code: 'PREFLIGHT_INCOMPLETE' });
    }
    await writeRemoteText(hostConfig, `${incoming}/expected-baseline`, plan.baseline);
    await writeRemoteText(hostConfig, `${txn}/old-frontend-port`, String(plan.oldFrontendPort || ''));
  } catch (error) {
    await runRemote(hostConfig, `rm -rf -- ${shellQuote(txn)}`, { timeoutMs: 60000 }).catch(() => {});
    throw error;
  }
  progress(52, '启动部署事务');
  await launchWorker(hostConfig, txn, 'deploy', workerArgs({
    installDir,
    deployDir,
    txn,
    plan,
    overrideFile,
    oldOverrideFile
  }));
  const transaction = await pollTransaction(hostConfig, installDir, 12 * 60 * 60 * 1000, progress);
  const log = await readWorkerTail(hostConfig, txn);
  const success = transaction.state === 'committed';
  const recovered = transaction.state === 'recovered' && plan.hadDeployment && transaction.deploymentStarted;
  const firstDeployCleaned = transaction.state === 'recovered' && !plan.hadDeployment;
  const recoveryFailed = transaction.state === 'recovery_failed';
  if (success) {
    await runRemote(hostConfig, `rm -rf -- ${shellQuote(incoming)}`, { timeoutMs: 60000 }).catch(() => {});
  }
  const output = success
    ? `完整版本部署成功：${bundle.version()}；事务 ${transaction.id}\n${log}`
    : !transaction.deploymentStarted
      ? `本次部署在修改目标前已停止，目标文件与数据未被本次部署修改。失败原因：${transaction.error || '请查看执行详情'}\n${log}`
    : recovered
      ? `新版本部署失败，已恢复上一版本 ${plan.previousVersion || '未知版本'}\n失败原因：${transaction.error || '请查看事务日志'}\n${log}`
      : firstDeployCleaned
        ? `首次部署失败，已清理本次创建的服务与数据卷\n失败原因：${transaction.error || '请查看事务日志'}\n${log}`
      : recoveryFailed
        ? `新版本部署失败，自动恢复未完成。现场保留在 ${transaction.snapshotDir}\n${log}`
        : `部署事务结束但状态未确认：${transaction.state}\n${log}`;
  progress(100, success ? '部署完成' : recovered ? '已恢复上一版本' : '部署未完成');
  return resultFromTransaction(transaction, {
    success,
    recovered,
    recoveryFailed,
    error: transaction.error,
    output,
    durationSecs: Math.round((Date.now() - started) / 1000)
  });
}

export async function recoverVersionRemote(profile, hostConfig, options = {}) {
  const started = Date.now();
  const installDir = trimSlashes(validateInstallDir(options.installDir || hostConfig.installDir || profile.installDir));
  const transaction = await inspectRemoteTransaction(hostConfig, installDir);
  if (!transaction) throw new DeployError('目标机没有可恢复的部署事务', { code: 'TRANSACTION_NOT_FOUND' });
  if (transaction.state === 'recovery_failed') {
    throw new DeployError(
      `该事务上次恢复失败，请按快照目录 ${transaction.snapshotDir} 人工处理，系统不会自动重试`,
      { code: 'RECOVERY_ALREADY_FAILED' }
    );
  }
  if (transaction.state === 'recovering') {
    throw new DeployError('该事务正在恢复或上次恢复被中断，请先检查事务现场后再处理', {
      code: 'RECOVERY_IN_PROGRESS'
    });
  }
  if (!BLOCKING_STATES.has(transaction.state)) {
    throw new DeployError(`事务 ${transaction.id} 当前状态为 ${transaction.state}，无需恢复`, {
      code: 'TRANSACTION_NOT_RECOVERABLE'
    });
  }
  if (await remoteDeployLockActive(hostConfig, installDir)) {
    throw new DeployError('目标机部署任务仍在执行，请等待事务结束或确认进程已退出后再恢复', {
      code: 'DEPLOY_IN_PROGRESS'
    });
  }
  if (transaction.projectName && await remoteProjectLockActive(hostConfig, transaction.projectName)) {
    throw new DeployError('同一 Compose 项目正在其他部署目录执行任务，暂不能恢复', {
      code: 'COMPOSE_PROJECT_CONFLICT'
    });
  }
  const txn = `${installDir}/.deploy-desktop/transactions/${transaction.id}`;
  const check = await runRemote(hostConfig, `test -f ${shellQuote(`${txn}/worker.sh`)} && test -f ${shellQuote(`${txn}/incoming/volume-specs.tsv`)} && printf 'READY\\n'`, {
    timeoutMs: 60000,
    throwOnError: false
  });
  if (!check.stdout.includes('READY')) {
    throw new DeployError(`部署事务缺少恢复脚本或快照清单，现场保留在 ${transaction.snapshotDir}`, {
      code: 'TRANSACTION_RECOVERY_DATA_MISSING'
    });
  }
  const plan = {
    hadDeployment: transaction.hadDeployment,
    oldCompose: transaction.previousCompose,
    newCompose: transaction.newCompose,
    services: transaction.services,
    oldServices: transaction.previousServices,
    frontendService: transaction.frontendService,
    frontendPort: transaction.frontendPort
  };
  await writeRemoteText(hostConfig, `${txn}/project-name`, transaction.projectName || '');
  await writeRemoteText(hostConfig, `${txn}/project-lock`, projectLockPath(transaction.projectName));
  await launchWorker(hostConfig, txn, 'recover', workerArgs({
    installDir,
    deployDir: transaction.deployDir,
    txn,
    plan,
    overrideFile: `${txn}/image-override.compose.json`,
    oldOverrideFile: `${txn}/old-image-override.compose.json`
  }));
  const finalTransaction = await pollTransaction(
    hostConfig,
    installDir,
    6 * 60 * 60 * 1000,
    options.onProgress,
    { state: transaction.state, updatedAt: transaction.updatedAt }
  );
  const log = await readWorkerTail(hostConfig, txn);
  const recovered = finalTransaction.state === 'recovered' && transaction.hadDeployment && finalTransaction.deploymentStarted;
  const firstDeployCleaned = finalTransaction.state === 'recovered' && !transaction.hadDeployment;
  return resultFromTransaction(finalTransaction, {
    success: false,
    recovered,
    recoveryFailed: finalTransaction.state !== 'recovered',
    error: recovered || firstDeployCleaned
      ? finalTransaction.error
      : (finalTransaction.recoveryError || '远端恢复未完成'),
    output: !finalTransaction.deploymentStarted
      ? `本次事务在修改目标前已停止，无需恢复服务、文件或数据卷\n事务：${finalTransaction.id}\n${log}`
      : recovered
      ? `已恢复上一版本 ${finalTransaction.previousVersion || '未知版本'}\n事务：${finalTransaction.id}\n${log}`
      : firstDeployCleaned
        ? `首次部署事务已清理，未保留上一版本\n${log}`
      : `恢复未完成，现场保留在 ${finalTransaction.snapshotDir}\n${log}`,
    durationSecs: Math.round((Date.now() - started) / 1000)
  });
}

async function remoteDeployLockActive(hostConfig, installDir) {
  const lock = `${trimSlashes(installDir)}/.deploy-desktop/deploy.lock`;
  const result = await runRemote(hostConfig, `command -v flock >/dev/null 2>&1 || exit 2
if [ -f ${shellQuote(lock)} ] && ! flock -n ${shellQuote(lock)} -c true; then
  printf 'ACTIVE\\n'
fi
`, { timeoutMs: 60000 });
  return result.stdout.includes('ACTIVE');
}

async function remoteProjectLockActive(hostConfig, projectName) {
  const lock = projectLockPath(projectName);
  const result = await runRemote(hostConfig, `command -v flock >/dev/null 2>&1 || exit 2
if [ -f ${shellQuote(lock)} ] && ! flock -n ${shellQuote(lock)} -c true; then
  printf 'ACTIVE\\n'
fi
`, { timeoutMs: 60000 });
  return result.stdout.includes('ACTIVE');
}
