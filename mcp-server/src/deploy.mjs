import fs from 'node:fs/promises';
import fsSync from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import dgram from 'node:dgram';
import { DeployError } from './errors.mjs';
import { commandExists, requireBinary, run } from './exec.mjs';
import { containerName, loadProfile, resolveContainer, resolveServiceKey } from './profile.mjs';
import { PROJECT_ROOT } from './config.mjs';
import { explainSshFailure, runRemote } from './ssh.mjs';
import { clamp, expandHome, formatBytes, nowStamp, shellQuote } from './util.mjs';

function memoryMb(bytes) {
  return Math.round(bytes / 1024 / 1024);
}

async function osRelease() {
  if (process.platform === 'darwin') {
    const result = await run('sw_vers', ['-productVersion'], { timeoutMs: 10000 });
    return result.code === 0 ? `macOS ${result.stdout.trim()}` : 'macOS';
  }
  if (process.platform === 'linux') {
    try {
      const content = await fs.readFile('/etc/os-release', 'utf8');
      const line = content.split('\n').find((item) => item.startsWith('PRETTY_NAME='));
      if (line) return line.slice('PRETTY_NAME='.length).replaceAll('"', '');
    } catch {
      // fall through
    }
  }
  return `${os.type()} ${os.release()}`;
}

async function diskFreeGb(targetDir) {
  const checkPath = targetDir || (process.platform === 'linux' ? '/home' : '.');
  try {
    const stat = await fs.statfs(checkPath);
    return Math.round((stat.bavail * stat.bsize) / 1024 / 1024 / 1024 * 10) / 10;
  } catch {
    const result = await run('df', ['-Pk', checkPath], { timeoutMs: 10000 });
    if (result.code !== 0) return 0;
    const line = result.stdout.split('\n')[1];
    const parts = line?.split(/\s+/);
    return parts ? Math.round((Number(parts[3]) / 1024 / 1024) * 10) / 10 : 0;
  }
}

async function dockerStatusLocal() {
  if (!commandExists('docker')) return { installed: false, version: null, running: false, composeInstalled: false, composeVersion: null };
  const versionResult = await run('docker', ['--version'], { timeoutMs: 15000 });
  const version = versionResult.code === 0 ? versionResult.stdout.trim() : null;
  const info = await run('docker', ['info'], { timeoutMs: 20000 });
  const compose = await run('docker', ['compose', 'version'], { timeoutMs: 15000 });
  let composeVersion = compose.code === 0 ? compose.stdout.trim() : null;
  if (!composeVersion && commandExists('docker-compose')) {
    const legacy = await run('docker-compose', ['--version'], { timeoutMs: 15000 });
    composeVersion = legacy.code === 0 ? legacy.stdout.trim() : null;
  }
  return {
    installed: true,
    version,
    running: info.code === 0,
    composeInstalled: Boolean(composeVersion),
    composeVersion
  };
}

function checkTcpPort(port) {
  return new Promise((resolve) => {
    const server = net.createServer();
    server.once('error', (error) => resolve({ available: error.code !== 'EADDRINUSE', occupant: error.code === 'EADDRINUSE' ? 'TCP 端口监听中' : error.code }));
    server.once('listening', () => server.close(() => resolve({ available: true, occupant: null })));
    server.listen({ port, host: '0.0.0.0', exclusive: true });
  });
}

function checkUdpPort(port) {
  return new Promise((resolve) => {
    const socket = dgram.createSocket('udp4');
    socket.once('error', (error) => {
      socket.close(() => resolve({ available: error.code !== 'EADDRINUSE', occupant: error.code === 'EADDRINUSE' ? 'UDP 端口监听中' : error.code }));
    });
    socket.once('listening', () => socket.close(() => resolve({ available: true, occupant: null })));
    socket.bind({ port, address: '0.0.0.0', exclusive: true });
  });
}

export async function checkLocalEnv(profile, targetDir) {
  const docker = await dockerStatusLocal();
  const ports = [];
  for (const container of profile.containers) {
    if (!container.port) continue;
    const state = container.protocol === 'UDP' ? await checkUdpPort(container.port) : await checkTcpPort(container.port);
    ports.push({
      port: container.port,
      protocol: container.protocol,
      service: `${container.serviceKey} (${container.label})`,
      available: state.available,
      occupyingProcess: state.occupant
    });
  }
  const totalMemoryMb = memoryMb(os.totalmem());
  const availableMemoryMb = memoryMb(os.freemem());
  const diskAvailableGb = await diskFreeGb(targetDir);
  const issues = [];
  if (!docker.installed) issues.push('未检测到 Docker 运行环境');
  else if (!docker.running) issues.push('Docker 服务未处于运行状态');
  if (!docker.composeInstalled) issues.push('未检测到 Docker Compose 插件');
  if (totalMemoryMb > 0 && totalMemoryMb < 8192) issues.push(`宿主机物理内存为 ${totalMemoryMb}MB，建议至少 16GB`);
  if (diskAvailableGb > 0 && diskAvailableGb < 20) issues.push(`目标分区磁盘剩余空间仅 ${diskAvailableGb}GB，建议预留 30GB 以上`);
  for (const port of ports.filter((item) => !item.available)) {
    issues.push(`${port.protocol} 端口 ${port.port} 已被占用 (${port.occupyingProcess || '未知进程'})`);
  }
  return {
    os: process.platform,
    arch: os.arch(),
    osRelease: await osRelease(),
    cpuCores: os.cpus().length,
    totalMemoryMb,
    availableMemoryMb,
    diskAvailableGb,
    dockerInstalled: docker.installed,
    dockerVersion: docker.version,
    dockerRunning: docker.running,
    composeInstalled: docker.composeInstalled,
    composeVersion: docker.composeVersion,
    ports,
    isReady: docker.installed && docker.running && docker.composeInstalled,
    issues
  };
}

export async function checkRemoteEnv(hostConfig, profile) {
  const portList = profile.containers.filter((container) => container.port).map((container) => container.port);
  const script = `echo "OS=$(uname -s 2>/dev/null || echo Linux)"
echo "ARCH=$(uname -m 2>/dev/null || echo x86_64)"
OS_RELEASE_VALUE=$(sed -n 's/^PRETTY_NAME=//p' /etc/os-release 2>/dev/null | tr -d '"' | head -1)
[ -n "$OS_RELEASE_VALUE" ] || OS_RELEASE_VALUE=$(uname -s)
echo "OS_RELEASE=$OS_RELEASE_VALUE"
echo "CPU_CORES=$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null || echo 0)"
echo "MEM_TOTAL_KB=$(awk '/^MemTotal:/{print $2}' /proc/meminfo 2>/dev/null || echo 0)"
echo "MEM_AVAILABLE_KB=$(awk '/^MemAvailable:/{print $2}' /proc/meminfo 2>/dev/null || awk '/^MemFree:/{print $2}' /proc/meminfo 2>/dev/null || echo 0)"
echo "DISK_AVAILABLE_KB=$(df -Pk /home 2>/dev/null | awk 'NR==2{print $4}' || df -Pk / 2>/dev/null | awk 'NR==2{print $4}')"
echo "DOCKER_VERSION=$(docker --version 2>/dev/null || true)"
echo "COMPOSE_VERSION=$(docker compose version 2>/dev/null || docker-compose --version 2>/dev/null || true)"
echo "DOCKER_RUNNING=$(docker info >/dev/null 2>&1 && echo yes || echo no)"
for p in ${portList.join(' ')}; do
  if command -v ss >/dev/null 2>&1; then
    if ss -ltnu 2>/dev/null | awk '{print $5}' | grep -q ":$p$"; then echo "PORT_$p=busy"; else echo "PORT_$p=free"; fi
  elif command -v netstat >/dev/null 2>&1; then
    if netstat -ltnu 2>/dev/null | awk '{print $4}' | grep -q ":$p$"; then echo "PORT_$p=busy"; else echo "PORT_$p=free"; fi
  else
    echo "PORT_$p=unknown"
  fi
done
`;
  const result = await runRemote(hostConfig, script, { timeoutMs: 60000 });
  const values = {};
  for (const line of result.stdout.split('\n')) {
    const index = line.indexOf('=');
    if (index > 0) values[line.slice(0, index)] = line.slice(index + 1).trim();
  }
  const totalMemoryMb = Math.round((Number(values.MEM_TOTAL_KB) || 0) / 1024);
  const availableMemoryMb = Math.round((Number(values.MEM_AVAILABLE_KB) || 0) / 1024);
  const diskAvailableGb = Math.round(((Number(values.DISK_AVAILABLE_KB) || 0) / 1024 / 1024) * 10) / 10;
  const dockerInstalled = Boolean(values.DOCKER_VERSION);
  const composeInstalled = Boolean(values.COMPOSE_VERSION);
  const dockerRunning = values.DOCKER_RUNNING === 'yes';
  const ports = profile.containers.filter((container) => container.port).map((container) => {
    const state = values[`PORT_${container.port}`] || 'unknown';
    return {
      port: container.port,
      protocol: container.protocol,
      service: `${container.serviceKey} (${container.label})`,
      available: state !== 'busy',
      occupyingProcess: state === 'busy' ? '远程主机端口监听中' : null
    };
  });
  const issues = [];
  if (!dockerInstalled) issues.push('远程机器未安装 Docker');
  else if (!dockerRunning) issues.push('远程机器 Docker 服务未运行');
  if (!composeInstalled) issues.push('远程机器缺少 Docker Compose 插件');
  if (totalMemoryMb > 0 && totalMemoryMb < 8192) issues.push(`远程机器物理内存为 ${totalMemoryMb}MB，建议至少 16GB`);
  if (diskAvailableGb > 0 && diskAvailableGb < 20) issues.push(`远程部署分区磁盘剩余空间仅 ${diskAvailableGb}GB，建议预留 30GB 以上`);
  for (const port of ports.filter((item) => !item.available)) issues.push(`${port.protocol} 端口 ${port.port} 已被占用`);
  return {
    os: values.OS || 'Linux',
    arch: values.ARCH || 'x86_64',
    osRelease: values.OS_RELEASE || 'Linux',
    cpuCores: Number(values.CPU_CORES) || 0,
    totalMemoryMb,
    availableMemoryMb,
    diskAvailableGb,
    dockerInstalled,
    dockerVersion: values.DOCKER_VERSION || null,
    dockerRunning,
    composeInstalled,
    composeVersion: values.COMPOSE_VERSION || null,
    ports,
    isReady: dockerInstalled && dockerRunning && composeInstalled,
    issues,
    rawOutput: result.stdout
  };
}

function resolvedArtifacts(profile, installDir, deployDir) {
  return profile.artifacts.map((artifact) => ({
    ...artifact,
    target: path.resolve(artifact.root === 'install' ? installDir : deployDir, artifact.path)
  }));
}

export async function inspectLocalDeployment(profile, installDirInput, deployDirInput) {
  const installDir = path.resolve(expandHome(installDirInput || profile.installDir));
  const deployDir = path.resolve(expandHome(deployDirInput || path.join(installDir, profile.deploySourceRel || '')));
  const docker = await dockerStatusLocal();
  const artifacts = [];
  for (const artifact of resolvedArtifacts(profile, installDir, deployDir)) {
    let info;
    try {
      const stat = await fs.stat(artifact.target);
      info = {
        exists: artifact.kind === 'dir' ? stat.isDirectory() : stat.isFile(),
        path: artifact.target,
        sizeBytes: stat.isFile() ? stat.size : 0,
        modified: stat.mtime.toISOString()
      };
    } catch {
      info = { exists: false, path: artifact.target, sizeBytes: 0, modified: '' };
    }
    artifacts.push({ key: artifact.key, label: artifact.label, root: artifact.root, path: artifact.path, kind: artifact.kind, ...info });
  }
  const composeFile = await firstExisting([profile.compose.variantFile, profile.compose.file].filter(Boolean));
  const containers = [];
  for (const container of profile.containers) {
    const name = containerName(profile, container.serviceKey);
    const inspect = await run('docker', ['inspect', '--format', '{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}|{{.Config.Image}}', name], { timeoutMs: 10000 });
    if (inspect.code !== 0) {
      containers.push({ name, serviceKey: container.serviceKey, state: 'not_created', health: 'none', statusText: '容器未创建', ports: '', image: '' });
      continue;
    }
    const [state, health, image] = inspect.stdout.trim().split('|');
    const ps = await run('docker', ['ps', '-a', '--filter', `name=^${name}$`, '--format', '{{.Status}}\t{{.Ports}}'], { timeoutMs: 10000 });
    const psText = ps.code === 0 ? ps.stdout.trim() : '';
    const [statusText, ports] = psText.split('\t');
    containers.push({ name, serviceKey: container.serviceKey, state, health, statusText: statusText || '', ports: ports || '', image: image || '' });
  }
  const running = containers.filter((container) => container.state === 'running').length;
  const issues = [];
  if (!docker.installed) issues.push('未安装 Docker');
  else if (!docker.running) issues.push('Docker 服务未运行');
  if (!docker.composeInstalled) issues.push('缺少 Docker Compose');
  if (!composeFile) issues.push(`部署目录缺少 Compose 清单: ${deployDir}`);
  for (const artifact of artifacts) {
    if (!artifact.exists) issues.push(`缺少产物: ${artifact.label || artifact.key} (${artifact.target})`);
  }
  return {
    deployed: Boolean(composeFile),
    product: profile.product,
    version: profile.version,
    installDir,
    deployDir,
    composeFile,
    dockerAvailable: docker.installed,
    dockerRunning: docker.running,
    composeAvailable: docker.composeInstalled,
    artifacts,
    containers,
    runningContainers: running,
    totalContainers: containers.length,
    issues,
    message: composeFile ? `已部署 ${running}/${containers.length} 个容器运行中` : '未检测到部署目录'
  };
}

async function firstExisting(files) {
  for (const file of files) {
    if (await fs.access(file).then(() => true).catch(() => false)) return file;
  }
  return '';
}

export async function inspectRemoteDeployment(profile, hostConfig, installDirInput) {
  const installDir = String(installDirInput || hostConfig.installDir || profile.installDir).trim().replace(/\/+$/, '');
  const deployDir = [installDir, profile.deploySourceRel].filter(Boolean).join('/');
  const targets = resolvedArtifacts(profile, installDir, deployDir);
  const containerEntries = profile.containers.map((container) => ({
    serviceKey: container.serviceKey,
    name: containerName(profile, container.serviceKey)
  }));
  const artifactLines = targets.map((artifact, index) => `artifact ${index} ${shellQuote(artifact.target)}`).join('\n');
  const script = `set -e
INSTALL=${shellQuote(installDir)}
DEPLOY=${shellQuote(deployDir)}
echo "INSTALL_DIR=$INSTALL"
echo "DEPLOY_DIR=$DEPLOY"
if [ -d "$DEPLOY" ]; then echo "DEPLOY_EXISTS=yes"; else echo "DEPLOY_EXISTS=no"; fi
COMPOSE_MAIN=${shellQuote(profile.compose.fileRelative)}
COMPOSE_VARIANT=${shellQuote(profile.compose.variantRelative)}
if [ -n "$COMPOSE_VARIANT" ] && [ -f "$DEPLOY/$COMPOSE_VARIANT" ]; then
  echo "COMPOSE_FILE=$COMPOSE_VARIANT"
elif [ -f "$DEPLOY/$COMPOSE_MAIN" ]; then
  echo "COMPOSE_FILE=$COMPOSE_MAIN"
else
  echo "COMPOSE_FILE="
fi
if [ -f "$INSTALL/.deploy-desktop/deployed.json" ]; then echo "DEPLOYED_RECORD=yes"; else echo "DEPLOYED_RECORD=no"; fi
if command -v docker >/dev/null 2>&1; then echo "DOCKER=yes"; else echo "DOCKER=no"; fi
if docker info >/dev/null 2>&1; then echo "DOCKER_RUNNING=yes"; else echo "DOCKER_RUNNING=no"; fi
if docker compose version >/dev/null 2>&1; then echo "COMPOSE=yes"; elif command -v docker-compose >/dev/null 2>&1; then echo "COMPOSE=yes"; else echo "COMPOSE=no"; fi
artifact() {
  local key=$1
  local path=$2
  if [ -e "$path" ]; then
    echo "ART_\${key}_EXISTS=yes"
    if [ -f "$path" ]; then echo "ART_\${key}_SIZE=$(stat -c '%s' "$path" 2>/dev/null || echo 0)"; else echo "ART_\${key}_SIZE=0"; fi
    echo "ART_\${key}_MTIME=$(stat -c '%y' "$path" 2>/dev/null | cut -d. -f1)"
  else
    echo "ART_\${key}_EXISTS=no"
    echo "ART_\${key}_SIZE=0"
    echo "ART_\${key}_MTIME="
  fi
}
${artifactLines}
echo "---CONTAINERS---"
for c in ${containerEntries.map((entry) => shellQuote(entry.name)).join(' ')}; do
  state=$(docker inspect --format '{{.State.Status}}' "$c" 2>/dev/null || echo missing)
  health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$c" 2>/dev/null || echo none)
  image=$(docker inspect --format '{{.Config.Image}}' "$c" 2>/dev/null || echo "")
  line=$(docker ps -a --filter "name=^$c$" --format '{{.Status}}|{{.Ports}}' 2>/dev/null | head -1)
  status_text=\${line%%|*}
  ports=\${line#*|}
  if [ "$ports" = "$line" ]; then ports=""; fi
  printf 'CONTAINER|%s|%s|%s|%s|%s|%s\n' "$c" "$state" "$health" "$image" "$status_text" "$ports"
done
`;
  const result = await runRemote(hostConfig, script, { timeoutMs: 60000 });
  const values = {};
  const containers = [];
  let section = '';
  for (const line of result.stdout.split('\n')) {
    if (line === '---CONTAINERS---') { section = 'containers'; continue; }
    if (section === 'containers' && line.startsWith('CONTAINER|')) {
      const [, name, state, health, image, statusText, ports] = line.split('|');
      containers.push({
        name,
        serviceKey: containerEntries.find((entry) => entry.name === name)?.serviceKey || name,
        state,
        health,
        image,
        statusText: statusText || '',
        ports: ports || ''
      });
      continue;
    }
    const index = line.indexOf('=');
    if (index > 0) values[line.slice(0, index)] = line.slice(index + 1);
  }
  const artifacts = targets.map((artifact, index) => {
    const info = {
      exists: values[`ART_${index}_EXISTS`] === 'yes',
      sizeBytes: Number(values[`ART_${index}_SIZE`]) || 0,
      modified: values[`ART_${index}_MTIME`] || ''
    };
    return { key: artifact.key, label: artifact.label, root: artifact.root, path: artifact.path, kind: artifact.kind, target: artifact.target, ...info };
  });
  const dockerAvailable = values.DOCKER === 'yes';
  const dockerRunning = values.DOCKER_RUNNING === 'yes';
  const composeAvailable = values.COMPOSE === 'yes';
  const composeFile = values.COMPOSE_FILE || '';
  const issues = [];
  if (values.DEPLOY_EXISTS !== 'yes') issues.push(`目标机部署目录不存在: ${deployDir}`);
  if (!dockerAvailable) issues.push('目标机未安装 Docker');
  else if (!dockerRunning) issues.push('目标机 Docker 服务未运行');
  if (!composeAvailable) issues.push('目标机缺少 Docker Compose');
  if (!composeFile) issues.push('目标机缺少 Compose 清单');
  for (const artifact of artifacts) {
    if (!artifact.exists) issues.push(`缺少产物: ${artifact.label || artifact.key} (${artifact.target})`);
  }
  const running = containers.filter((container) => container.state === 'running').length;
  return {
    deployed: values.DEPLOY_EXISTS === 'yes' && Boolean(composeFile),
    product: profile.product,
    version: profile.version,
    deployedRecord: values.DEPLOYED_RECORD === 'yes',
    installDir,
    deployDir,
    composeFile,
    dockerAvailable,
    dockerRunning,
    composeAvailable,
    artifacts,
    containers,
    runningContainers: running,
    totalContainers: containers.length,
    issues,
    message: composeFile ? `目标机 ${running}/${containers.length} 个容器运行中` : '目标机未部署'
  };
}

export async function composeAction(profile, options) {
  const target = options.target || 'local';
  const action = String(options.action || '').trim();
  const allowed = ['up', 'down', 'stop', 'restart', 'ps', 'pull'];
  if (!allowed.includes(action)) throw new DeployError(`不支持的 Compose 操作: ${action}`, { hint: `可用: ${allowed.join(', ')}` });
  const service = resolveServiceKey(profile, options.service);
  if (action === 'down' && options.confirm !== true) throw new DeployError('停止并删除容器需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
  const actionArg = action === 'up' ? `up -d${service ? ` ${shellQuote(service)}` : ''}`
    : action === 'pull' ? `pull${service ? ` ${shellQuote(service)}` : ''}`
    : action === 'ps' ? 'ps'
    : `${action}${service && action !== 'down' ? ` ${shellQuote(service)}` : ''}`;
  const main = profile.compose.fileRelative;
  const variant = profile.compose.variantRelative;
  if (target === 'remote') {
    const deployDir = String(options.deployDir || options.hostConfig?.deployDir || profile.deployDir).trim().replace(/\/+$/, '');
    const prefix = deployDir.endsWith(profile.deploySourceRel) && profile.deploySourceRel
      ? deployDir.slice(0, -profile.deploySourceRel.length).replace(/\/+$/, '')
      : (options.hostConfig?.installDir || profile.installDir);
    const script = `set -e
DEPLOY=${shellQuote([prefix, profile.deploySourceRel].filter(Boolean).join('/'))}
cd "$DEPLOY"
COMPOSE_MAIN=${shellQuote(main)}
COMPOSE_VARIANT=${shellQuote(variant)}
if [ -n "$COMPOSE_VARIANT" ] && [ -f "$COMPOSE_VARIANT" ]; then COMPOSE_FILE="$COMPOSE_VARIANT"
elif [ -f "$COMPOSE_MAIN" ]; then COMPOSE_FILE="$COMPOSE_MAIN"
else echo "目标目录未找到 Compose 清单: $COMPOSE_MAIN" >&2; exit 3; fi
if docker compose version >/dev/null 2>&1; then docker compose -f "$COMPOSE_FILE" ${actionArg}
else docker-compose -f "$COMPOSE_FILE" ${actionArg}; fi
`;
    const result = await runRemote(options.hostConfig, script, { timeoutMs: options.timeoutMs ?? 30 * 60 * 1000 });
    return { success: true, action, service: service || 'all', output: `${result.stdout}\n${result.stderr}`.trim() };
  }
  const deployDir = path.resolve(expandHome(options.deployDir || profile.deployDir));
  const file = await firstExisting([profile.compose.variantFile, profile.compose.file].filter(Boolean));
  if (!file) throw new DeployError(`部署目录未找到 Compose 清单: ${deployDir}`);
  const docker = requireBinary('docker');
  const args = ['compose', '-f', file, action];
  if (action === 'up') args.push('-d');
  if (service && action !== 'down') args.push(service);
  const result = await run(docker, args, { cwd: deployDir, timeoutMs: options.timeoutMs ?? 30 * 60 * 1000 });
  return {
    success: result.code === 0,
    action,
    service: service || 'all',
    output: `${result.stdout}\n${result.stderr}`.trim(),
    error: result.code === 0 ? undefined : `命令退出码 ${result.code}`
  };
}

export async function serviceLogs(profile, options) {
  const container = resolveContainer(profile, options.container || options.service);
  const tail = clamp(options.tail || 200, 1, 5000);
  if (options.target === 'remote') {
    const result = await runRemote(options.hostConfig, `docker logs --tail ${tail} ${shellQuote(container)} 2>&1\n`, {
      timeoutMs: 120000,
      throwOnError: false
    });
    if (result.code !== 0) throw new DeployError(result.stderr.trim() || '读取远程日志失败，目标容器可能不存在');
    return { container, tail, logs: result.stdout.trim() };
  }
  const docker = requireBinary('docker');
  const result = await run(docker, ['logs', '--tail', String(tail), container], { timeoutMs: 120000 });
  return { container, tail, logs: `${result.stdout}\n${result.stderr}`.trim(), success: result.code === 0 };
}

export async function dbCheck(profile, options) {
  const container = containerName(profile, profile.database.serviceKey);
  const client = profile.database.client;
  const port = profile.database.port;
  if (options.target === 'remote') {
    const result = await runRemote(options.hostConfig, `set -e
STATE=$(docker inspect --format '{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' ${shellQuote(container)} 2>/dev/null || echo "not_found|not_found")
echo "STATE=$STATE"
RUNNING=$(echo "$STATE" | cut -d'|' -f1)
if [ "$RUNNING" = "running" ]; then
  if docker exec ${shellQuote(container)} ${shellQuote(client)} -h127.0.0.1 -P${port} -uroot -e 'SELECT 1;' >/dev/null 2>&1; then echo "READY=yes"; else echo "READY=no"; fi
else echo "READY=no"; fi
`, { timeoutMs: 60000 });
    const state = result.stdout.split('\n').find((line) => line.startsWith('STATE='))?.slice(6) || 'not_found|not_found';
    const [stateValue, health] = state.split('|');
    const ready = result.stdout.includes('READY=yes');
    return {
      isRunning: stateValue === 'running',
      isReady: ready,
      containerHealth: health || 'not_found',
      message: stateValue !== 'running' ? 'OceanBase 容器未运行' : ready ? 'OceanBase 数据库连接正常' : 'OceanBase 正在启动初始化中'
    };
  }
  const docker = requireBinary('docker');
  const inspect = await run(docker, ['inspect', '--format', '{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}', container], { timeoutMs: 10000 });
  const [state, health] = inspect.code === 0 ? inspect.stdout.trim().split('|') : ['not_found', 'not_found'];
  let ready = false;
  if (state === 'running') {
    const query = await run(docker, ['exec', container, client, '-h127.0.0.1', `-P${port}`, '-uroot', '-e', 'SELECT 1;'], { timeoutMs: 30000 });
    ready = query.code === 0;
  }
  return {
    isRunning: state === 'running',
    isReady: ready,
    containerHealth: health,
    message: state !== 'running' ? 'OceanBase 容器未运行' : ready ? 'OceanBase 数据库连接正常' : 'OceanBase 正在启动初始化中'
  };
}

export async function dbImport(profile, options) {
  if (options.confirm !== true) {
    throw new DeployError('导入数据库会写入业务数据，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
  }
  const container = containerName(profile, profile.database.serviceKey);
  const client = profile.database.client;
  const port = profile.database.port;
  const database = profile.database.name;
  if (!options.sqlFile) throw new DeployError('缺少 sqlFile');
  if (options.target === 'remote') {
    let remotePath = options.remoteSqlPath;
    let cleanup = false;
    if (!remotePath) {
      const localPath = path.resolve(expandHome(options.sqlFile));
      const stat = await fs.stat(localPath).catch(() => null);
      if (!stat) throw new DeployError(`SQL 文件不存在: ${localPath}`);
      remotePath = `/tmp/deploy-desktop-${nowStamp()}-${path.basename(localPath)}`;
      await uploadFileForDb(options.hostConfig, localPath, remotePath);
      cleanup = true;
    }
    const script = `set -e
SQL=${shellQuote(remotePath)}
if [ ! -f "$SQL" ]; then echo "远程 SQL 脚本不存在: $SQL" >&2; exit 4; fi
docker exec ${shellQuote(container)} ${shellQuote(client)} -h127.0.0.1 -P${port} -uroot -e "CREATE DATABASE IF NOT EXISTS ${database} DEFAULT CHARACTER SET utf8mb4;"
docker exec -i ${shellQuote(container)} ${shellQuote(client)} -h127.0.0.1 -P${port} -uroot --default-character-set=utf8mb4 ${database} < "$SQL"
echo "TABLE_COUNT=$(docker exec ${shellQuote(container)} ${shellQuote(client)} -h127.0.0.1 -P${port} -uroot -N -e "SELECT count(*) FROM information_schema.tables WHERE table_schema='${database}';")"
echo "DICT_COUNT=$(docker exec ${shellQuote(container)} ${shellQuote(client)} -h127.0.0.1 -P${port} -uroot ${database} -N -e 'SELECT count(*) FROM parameter_dict;' 2>/dev/null || echo 0)"
echo "USER_COUNT=$(docker exec ${shellQuote(container)} ${shellQuote(client)} -h127.0.0.1 -P${port} -uroot ${database} -N -e 'SELECT count(*) FROM users;' 2>/dev/null || echo 0)"
docker exec ${shellQuote(container)} ${shellQuote(client)} -h127.0.0.1 -P${port} -uroot -N -e "SELECT table_name FROM information_schema.tables WHERE table_schema='${database}' ORDER BY table_name;" | while read -r table_name; do
  [ -n "$table_name" ] && echo "TABLE_NAME=$table_name"
done
`;
    try {
      const result = await runRemote(options.hostConfig, script, { timeoutMs: 60 * 60 * 1000 });
      return parseDbImport(result.stdout);
    } finally {
      if (cleanup) await runRemote(options.hostConfig, `rm -f ${shellQuote(remotePath)}\n`, { timeoutMs: 30000, throwOnError: false }).catch(() => {});
    }
  }
  const docker = requireBinary('docker');
  const localPath = path.resolve(expandHome(options.sqlFile));
  await fs.access(localPath).catch(() => { throw new DeployError(`SQL 文件不存在: ${localPath}`); });
  const create = await run(docker, ['exec', container, client, '-h127.0.0.1', `-P${port}`, '-uroot', '-e', `CREATE DATABASE IF NOT EXISTS ${database} DEFAULT CHARACTER SET utf8mb4;`], { timeoutMs: 60000 });
  if (create.code !== 0) throw new DeployError(`创建数据库失败: ${create.stderr.trim()}`);
  const sql = await fs.readFile(localPath);
  const imported = await run(docker, ['exec', '-i', container, client, '-h127.0.0.1', `-P${port}`, '-uroot', '--default-character-set=utf8mb4', database], {
    input: sql,
    timeoutMs: 60 * 60 * 1000
  });
  if (imported.code !== 0) throw new DeployError(`导入 SQL 失败: ${imported.stderr.trim()}`);
  const counts = await run(docker, ['exec', container, client, '-h127.0.0.1', `-P${port}`, '-uroot', '-N', '-e',
    `SELECT (SELECT count(*) FROM information_schema.tables WHERE table_schema='${database}'), (SELECT count(*) FROM ${database}.parameter_dict), (SELECT count(*) FROM ${database}.users);`], { timeoutMs: 60000 });
  const [tableCount, dictionaryCount, userCount] = counts.code === 0
    ? counts.stdout.trim().split(/\s+/).map((value) => Number(value) || 0)
    : [0, 0, 0];
  const tablesResult = await run(docker, ['exec', container, client, '-h127.0.0.1', `-P${port}`, '-uroot', '-N', '-e',
    `SELECT table_name FROM information_schema.tables WHERE table_schema='${database}' ORDER BY table_name;`], { timeoutMs: 60000 });
  const tables = tablesResult.code === 0 ? tablesResult.stdout.split('\n').map((line) => line.trim()).filter(Boolean) : [];
  return {
    success: true,
    tableCount,
    dictionaryCount,
    userCount,
    tables,
    message: `数据库迁移完成：${tableCount} 张表、${dictionaryCount} 条字典数据、${userCount} 个用户`
  };
}

async function uploadFileForDb(hostConfig, localPath, remotePath) {
  const { uploadFile } = await import('./ssh.mjs');
  return uploadFile(hostConfig, localPath, remotePath, { timeoutMs: 60 * 60 * 1000 });
}

function parseDbImport(stdout) {
  let tableCount = 0;
  let dictionaryCount = 0;
  let userCount = 0;
  const tables = [];
  for (const line of stdout.split('\n')) {
    if (line.startsWith('TABLE_COUNT=')) tableCount = Number(line.slice(12)) || 0;
    else if (line.startsWith('DICT_COUNT=')) dictionaryCount = Number(line.slice(11)) || 0;
    else if (line.startsWith('USER_COUNT=')) userCount = Number(line.slice(11)) || 0;
    else if (line.startsWith('TABLE_NAME=')) tables.push(line.slice(11).trim());
  }
  return {
    success: true,
    tableCount,
    dictionaryCount,
    userCount,
    tables,
    message: `数据库迁移完成：${tableCount} 张表、${dictionaryCount} 条字典数据、${userCount} 个用户`
  };
}

export async function installDocker(profile, options) {
  if (options.confirm !== true) throw new DeployError('安装 Docker 会修改系统文件，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
  const bundleDir = String(options.bundleDir || profile.bundleDir).replace(/\/+$/, '');
  if (options.target === 'remote') {
    const script = `set -e
ENGINE=${shellQuote(`${bundleDir}/docker-engine`)}
[ -d "$ENGINE" ] || { echo "未找到离线安装目录: $ENGINE" >&2; exit 3; }
echo "STEP|1|success|找到离线资源目录: $ENGINE"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
TGZ=$(find "$ENGINE" -maxdepth 1 -type f \\( -name '*.tgz' -o -name '*.tar.gz' \\) | head -1)
if [ -n "$TGZ" ]; then
  tar -zxf "$TGZ" -C "$TMP"
  SRC="$TMP/docker"
else
  SRC="$ENGINE"
fi
[ -d "$SRC" ] || { echo "未找到可安装的 Docker 二进制目录" >&2; exit 4; }
cp -a "$SRC"/. /usr/bin/
chmod +x /usr/bin/docker /usr/bin/dockerd /usr/bin/containerd /usr/bin/runc 2>/dev/null || true
mkdir -p /usr/local/bin /usr/libexec/docker/cli-plugins
if [ -f "$ENGINE/docker-compose" ]; then
  cp -a "$ENGINE/docker-compose" /usr/local/bin/docker-compose
  chmod +x /usr/local/bin/docker-compose
  cp -a "$ENGINE/docker-compose" /usr/libexec/docker/cli-plugins/docker-compose
  chmod +x /usr/libexec/docker/cli-plugins/docker-compose
fi
echo "STEP|2|success|Docker 与 Compose 二进制部署完成"
mkdir -p /etc/systemd/system /etc/docker
cat > /etc/systemd/system/docker.service <<'SERVICE'
[Unit]
Description=Docker Application Container Engine
Documentation=https://docs.docker.com
After=network-online.target firewalld.service containerd.service
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/bin/dockerd
ExecReload=/bin/kill -s HUP $MAINPID
TimeoutSec=0
RestartSec=2
Restart=always
LimitNOFILE=infinity
LimitNPROC=infinity
LimitCORE=infinity
TasksMax=infinity
Delegate=yes
KillMode=process
OOMScoreAdjust=-500

[Install]
WantedBy=multi-user.target
SERVICE
cat > /etc/docker/daemon.json <<'DAEMON'
{
  "log-driver": "json-file",
  "log-opts": { "max-size": "100m", "max-file": "3" },
  "live-restore": true
}
DAEMON
echo "STEP|3|success|Systemd 单元配置及日志策略写入完成"
systemctl daemon-reload
systemctl enable --now docker
sleep 2
docker --version
docker info >/dev/null
echo "STEP|4|success|Docker 守护进程启动并验证成功"
`;
    const result = await runRemote(options.hostConfig, script, { timeoutMs: 10 * 60 * 1000 });
    const steps = result.stdout.split('\n').filter((line) => line.startsWith('STEP|')).map((line) => {
      const [, index, status, message] = line.split('|');
      return { step: index, status, message };
    });
    return { success: true, steps, dockerVersion: result.stdout.split('\n').find((line) => line.includes('Docker version')) || null };
  }
  if (process.platform !== 'linux') throw new DeployError(`本地离线安装 Docker 仅适用于 Linux，当前系统为 ${process.platform}`);
  if (process.getuid?.() !== 0) throw new DeployError('本地安装 Docker 需要 root 权限，请使用 sudo 运行 MCP 服务');
  const engineDir = path.join(bundleDir, 'docker-engine');
  await fs.access(engineDir).catch(() => { throw new DeployError(`未找到离线安装目录: ${engineDir}`); });
  const entries = await fs.readdir(engineDir);
  const tgz = entries.find((name) => name.endsWith('.tgz') || name.endsWith('.tar.gz'));
  const steps = [];
  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'deploy-docker-'));
  try {
    if (tgz) {
      await run('tar', ['-zxf', path.join(engineDir, tgz), '-C', tmp], { timeoutMs: 120000 });
      await run('cp', ['-a', path.join(tmp, 'docker', '.'), '/usr/bin/'], { timeoutMs: 120000 });
    } else {
      await run('cp', ['-a', path.join(engineDir, '.'), '/usr/bin/'], { timeoutMs: 120000 });
    }
    steps.push({ step: '1', status: 'success', message: 'Docker 二进制已复制到 /usr/bin' });
    if (entries.includes('docker-compose')) {
      await fs.mkdir('/usr/local/bin', { recursive: true });
      await fs.mkdir('/usr/libexec/docker/cli-plugins', { recursive: true });
      await fs.copyFile(path.join(engineDir, 'docker-compose'), '/usr/local/bin/docker-compose');
      await fs.chmod('/usr/local/bin/docker-compose', 0o755);
      await fs.copyFile(path.join(engineDir, 'docker-compose'), '/usr/libexec/docker/cli-plugins/docker-compose');
      await fs.chmod('/usr/libexec/docker/cli-plugins/docker-compose', 0o755);
    }
    steps.push({ step: '2', status: 'success', message: 'Docker Compose 已安装' });
    await fs.mkdir('/etc/systemd/system', { recursive: true });
    await fs.writeFile('/etc/systemd/system/docker.service', `[Unit]
Description=Docker Application Container Engine
Documentation=https://docs.docker.com
After=network-online.target firewalld.service containerd.service
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/bin/dockerd
ExecReload=/bin/kill -s HUP $MAINPID
TimeoutSec=0
RestartSec=2
Restart=always
LimitNOFILE=infinity
LimitNPROC=infinity
LimitCORE=infinity
TasksMax=infinity
Delegate=yes
KillMode=process
OOMScoreAdjust=-500

[Install]
WantedBy=multi-user.target
`);
    await fs.mkdir('/etc/docker', { recursive: true });
    await fs.writeFile('/etc/docker/daemon.json', `${JSON.stringify({ 'log-driver': 'json-file', 'log-opts': { 'max-size': '100m', 'max-file': '3' }, 'live-restore': true }, null, 2)}\n`);
    steps.push({ step: '3', status: 'success', message: 'systemd 与 daemon.json 配置完成' });
    await run('systemctl', ['daemon-reload'], { timeoutMs: 60000 });
    await run('systemctl', ['enable', '--now', 'docker'], { timeoutMs: 120000 });
    await new Promise((resolve) => setTimeout(resolve, 2000));
    const info = await run('docker', ['info'], { timeoutMs: 30000 });
    if (info.code !== 0) throw new DeployError('Docker 守护进程启动超时，请使用 journalctl -u docker 检查日志');
    steps.push({ step: '4', status: 'success', message: 'Docker 守护进程启动并验证成功' });
    return { success: true, steps, dockerVersion: (await run('docker', ['--version'], { timeoutMs: 15000 })).stdout.trim() };
  } finally {
    await fs.rm(tmp, { recursive: true, force: true }).catch(() => {});
  }
}

export function findDeployScript() {
  const candidates = [];
  let dir = process.cwd();
  for (let i = 0; i < 4; i += 1) {
    candidates.push(path.join(dir, 'deploy-lan.sh'));
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  if (process.env.DEPLOY_DESKTOP_ROOT) candidates.push(path.join(process.env.DEPLOY_DESKTOP_ROOT, 'deploy-lan.sh'));
  candidates.push(path.join(os.homedir(), 'deploy/deploy-lan.sh'));
  candidates.push(path.join(PROJECT_ROOT, 'deploy-lan.sh'));
  return candidates.find((candidate) => {
    try { return fsSync.statSync(candidate).isFile(); } catch { return false; }
  }) || null;
}

export async function runRemoteDeploy(profile, hostConfig, options) {
  const mode = String(options.mode || '').trim();
  const modes = {
    'deploy-all': '--all',
    'server-only': '--server-only',
    'gateway-app-only': '--gateway-app-only',
    'frontend-only': '--frontend-only'
  };
  if (!modes[mode]) throw new DeployError(`不支持的远程部署模式: ${mode}`, { hint: `可用: ${Object.keys(modes).join(', ')}` });
  if (options.confirm !== true) throw new DeployError('远程构建/同步部署会修改目标机，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
  const script = findDeployScript();
  if (!script) {
    throw new DeployError('未找到 deploy-lan.sh，无法执行构建/同步类部署', {
      code: 'DEPLOY_SCRIPT_NOT_FOUND',
      hint: '请在包含源码和构建环境的仓库根目录运行 MCP 服务，或设置 DEPLOY_DESKTOP_ROOT。'
    });
  }
  const deployDir = String(options.deployDir || hostConfig.deployDir || `${hostConfig.installDir}/deploy`).replace(/\/+$/, '');
  const lanRoot = deployDir.endsWith('/deploy') ? deployDir.slice(0, -'/deploy'.length) : deployDir;
  const args = [script, '--host', hostConfig.host, '--user', hostConfig.user, '--port', String(hostConfig.port), '--dir', lanRoot];
  if (hostConfig.keyPath) args.push('--key', hostConfig.keyPath);
  args.push(modes[mode]);
  const result = await run('bash', args, { timeoutMs: options.timeoutMs ?? 60 * 60 * 1000 });
  return {
    success: result.code === 0,
    mode,
    output: `${result.stdout}\n${result.stderr}`.trim(),
    error: result.code === 0 ? undefined : `deploy-lan.sh 退出码 ${result.code}`
  };
}
