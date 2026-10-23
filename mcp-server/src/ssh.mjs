import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { loadConfig, validateHost } from './config.mjs';
import { DeployError } from './errors.mjs';
import { commandExists, findBinary, requireBinary, run } from './exec.mjs';
import { activeProject, loadConnections } from './projects.mjs';
import { expandHome, shellQuote } from './util.mjs';
import { BASH_STDIN, stdinProgram, bashCommand, transferScript } from './shell.mjs';

const DEFAULT_KEY_CANDIDATES = ['id_ed25519', 'id_rsa', 'id_ecdsa'];

function isLikelyHostAddress(value) {
  return /^[A-Za-z0-9._:[\]-]+$/.test(value);
}

async function managedHostByKey(key) {
  const { hosts, credentials } = await loadConnections();
  const target = String(key || '').trim();
  if (!target) return null;
  const host = hosts.find((item) => item.id === target)
    || hosts.find((item) => item.name === target)
    || hosts.find((item) => item.address === target);
  if (!host) return null;
  const credential = credentials.find((item) => item.id === host.credentialId) || null;
  return {
    id: host.id,
    name: host.name || host.address || host.id,
    kind: host.kind || 'ssh',
    host: host.address || '',
    port: Number(host.port) || 22,
    user: credential?.username || 'root',
    keyPath: credential?.privateKeyPath || '',
    installDir: '',
    deployDir: '',
    bundleDir: ''
  };
}

export async function resolveHost(input, overrides = {}) {
  const { config } = await loadConfig();
  const projectState = await activeProject().catch(() => null);
  const project = projectState?.project || null;
  const key = String(input || '').trim();
  const configured = key ? config.hosts[key] : null;
  const managed = await managedHostByKey(key).catch(() => null);
  const projectManaged = !key && project?.hostId ? await managedHostByKey(project.hostId).catch(() => null) : null;

  let base;
  if (managed && String(managed.kind).toLowerCase() !== 'local') {
    base = { ...managed };
  } else if (configured) {
    base = { ...configured };
  } else if (projectManaged) {
    base = { ...projectManaged };
  } else {
    base = {
      name: isLikelyHostAddress(key) ? key : '',
      host: isLikelyHostAddress(key) ? key : '',
      port: 22,
      user: 'root'
    };
  }
  if (!base.host) {
    throw new DeployError(key ? `未找到主机配置: ${key}` : '当前项目未绑定远程主机', {
      hint: '请在部署舱的“设置 → 管理主机”中添加主机，并为项目选择该主机。'
    });
  }
  for (const field of ['port', 'user', 'keyPath', 'installDir', 'deployDir', 'bundleDir', 'strictHostKeyChecking']) {
    if (overrides[field] !== undefined && overrides[field] !== null && String(overrides[field]).trim() !== '') {
      base[field] = overrides[field];
    }
  }
  const installDir = base.installDir || project?.installDir || '';
  if (installDir) base.installDir = expandHome(installDir);
  base.deployDir = base.deployDir
    ? expandHome(base.deployDir)
    : (base.installDir ? `${base.installDir.replace(/\/+$/, '')}/deploy` : '');
  const remoteHost = Boolean(base.host) && String(base.kind || 'ssh').toLowerCase() !== 'local';
  base.bundleDir = expandHome(base.bundleDir || (remoteHost ? '' : project?.lastBundleDir || ''));
  if (!base.bundleDir && remoteHost && base.installDir) {
    base.bundleDir = `${base.installDir.replace(/\/+$/, '')}/bundle`;
  }
  base.keyPath = base.keyPath ? expandHome(base.keyPath) : '';
  return { host: validateHost(base), configured: Boolean(configured || managed) };
}

export function resolveDefaultKey() {
  if (process.env.DEPLOY_DESKTOP_SSH_KEY) return expandHome(process.env.DEPLOY_DESKTOP_SSH_KEY);
  for (const name of DEFAULT_KEY_CANDIDATES) {
    const candidate = path.join(process.env.HOME || '', '.ssh', name);
    if (candidate && fs.existsSync(candidate)) return candidate;
  }
  return '';
}

export function ensureKeyPermissions(keyPath) {
  if (!keyPath || process.platform === 'win32') return;
  const stat = fs.statSync(keyPath);
  const mode = stat.mode & 0o777;
  if ((mode & 0o077) !== 0) {
    throw new DeployError(`SSH 私钥权限过宽: ${keyPath} (当前 ${mode.toString(8)})`, {
      code: 'INSECURE_KEY_PERMISSIONS',
      hint: `请执行 chmod 600 ${keyPath} 后重试。`
    });
  }
}

export function buildSshArgs(hostConfig, extra = []) {
  const keyPath = hostConfig.keyPath || resolveDefaultKey();
  if (keyPath) {
    if (!fs.existsSync(keyPath)) throw new DeployError(`SSH 私钥不存在: ${keyPath}`);
    ensureKeyPermissions(keyPath);
  }
  const args = [
    '-o', 'BatchMode=yes',
    '-o', 'ConnectTimeout=10',
    '-o', `StrictHostKeyChecking=${hostConfig.strictHostKeyChecking || 'accept-new'}`,
    '-o', 'LogLevel=ERROR',
    '-o', 'ServerAliveInterval=15',
    '-o', 'ServerAliveCountMax=8',
    '-p', String(hostConfig.port || 22)
  ];
  if (keyPath) args.push('-i', keyPath);
  args.push(`${hostConfig.user || 'root'}@${hostConfig.host}`, ...extra);
  return args;
}

export async function runRemote(hostConfig, script, options = {}) {
  const ssh = requireBinary('ssh');
  const result = await run(ssh, buildSshArgs(hostConfig, [BASH_STDIN]), {
    input: stdinProgram(script),
    timeoutMs: options.timeoutMs ?? 10 * 60 * 1000,
    maxOutputBytes: options.maxOutputBytes,
    onStdout: options.onStdout,
    onStderr: options.onStderr
  });
  if ((result.code !== 0 || result.timedOut || result.stdoutTruncated || result.stderrTruncated) && options.throwOnError !== false) {
    throw new DeployError(result.timedOut ? '远程执行超时，请检查目标状态后重试'
      : result.stdoutTruncated || result.stderrTruncated ? '远程输出超过限制，请检查目标状态与日志'
      : result.code === 255 ? explainSshFailure(result.stderr || result.stdout, hostConfig)
      : '目标机命令执行失败，请查看执行详情并检查部署配置', {
      code: result.timedOut ? 'SSH_TIMEOUT' : 'SSH_COMMAND_FAILED',
      details: {
        host: hostConfig.host,
        exitCode: result.code,
        stderr: result.stderr.trim() || undefined
      }
    });
  }
  return result;
}

export function explainSshFailure(raw, hostConfig) {
  const text = String(raw || '').trim();
  const lower = text.toLowerCase();
  let hint = '';
  if (lower.includes('no route to host')) {
    hint = '无法路由到目标主机，请确认工作站与目标机在同一局域网。';
  } else if (lower.includes('connection refused')) {
    hint = '目标主机可达，但 SSH 端口未监听或端口号错误。';
  } else if (lower.includes('permission denied')) {
    hint = hostConfig?.keyPath
      ? '认证被拒绝：目标机未授权当前密钥，请先下发免密公钥。'
      : '认证被拒绝：未找到可用私钥，请配置 keyPath 或先下发免密公钥。';
  } else if (lower.includes('timed out') || lower.includes('timeout')) {
    hint = '连接超时：请检查网络连通性与目标机防火墙。';
  } else if (lower.includes('host key verification failed')) {
    hint = '目标主机指纹与 known_hosts 不一致，请先清理旧指纹后重试。';
  } else if (lower.includes('command not found: ssh') || lower.includes('ssh: command not found')) {
    hint = '系统未找到 ssh 命令，请确认 OpenSSH 客户端已安装。';
  } else if (lower.includes('no such file or directory')) {
    hint = '目标机上不存在命令需要的文件或目录，请确认部署目录与实际安装目录一致。';
  }
  if (!text && !hint) return 'SSH 连接失败，未返回错误信息。';
  return hint ? `${hint}${text ? `\n原始错误: ${text}` : ''}` : `SSH 命令失败: ${text}`;
}

export async function testSshConnection(hostConfig) {
  const started = Date.now();
  const result = await run(requireBinary('ssh'), buildSshArgs(hostConfig, ['echo PONG_DEPLOY_DESKTOP']), {
    timeoutMs: 30000
  });
  const elapsed = Date.now() - started;
  const ok = result.code === 0 && result.stdout.includes('PONG_DEPLOY_DESKTOP');
  return {
    success: ok,
    host: hostConfig.host,
    user: hostConfig.user,
    latencyMs: ok ? elapsed : null,
    message: ok
      ? `SSH 连接测试成功 (${elapsed}ms)`
      : explainSshFailure(result.stderr || result.stdout, hostConfig)
  };
}

function authorizedKeysScript(publicKey) {
  return `set -e
umask 077
mkdir -p ~/.ssh
chmod 700 ~/.ssh
touch ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
KEY=${shellQuote(publicKey)}
grep -qxF "$KEY" ~/.ssh/authorized_keys || printf '%s\\n' "$KEY" >> ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
echo KEY_INSTALLED
`;
}

export async function setupSshKey(hostConfig, password = '') {
  const keyPath = hostConfig.keyPath || resolveDefaultKey() || path.join(process.env.HOME || '', '.ssh', 'id_ed25519');
  if (!fs.existsSync(keyPath)) {
    await fsp.mkdir(path.dirname(keyPath), { recursive: true, mode: 0o700 });
    const sshKeygen = requireBinary('ssh-keygen');
    const generated = await run(sshKeygen, ['-t', 'ed25519', '-N', '', '-f', keyPath, '-C', `deploy-desktop@${hostConfig.host}`], {
      timeoutMs: 30000
    });
    if (generated.code !== 0) {
      throw new DeployError(`生成 SSH 密钥失败: ${generated.stderr.trim()}`);
    }
  }
  ensureKeyPermissions(keyPath);
  const updated = { ...hostConfig, keyPath };
  const existing = await testSshConnection(updated);
  if (existing.success) {
    return {
      success: true,
      host: hostConfig.host,
      user: hostConfig.user,
      keyPath,
      message: `当前密钥已可免密登录，无需重复部署。本地私钥: ${keyPath}`,
      publicKey: (await fsp.readFile(`${keyPath}.pub`, 'utf8')).trim()
    };
  }
  const publicKey = (await fsp.readFile(`${keyPath}.pub`, 'utf8')).trim();
  if (!password) {
    return {
      success: false,
      host: hostConfig.host,
      user: hostConfig.user,
      keyPath,
      publicKey,
      message: `未填写初始密码，无法自动下发公钥。请手动把公钥追加到目标机 ~/.ssh/authorized_keys:\n${publicKey}`
    };
  }
  if (!commandExists('sshpass')) {
    return {
      success: false,
      host: hostConfig.host,
      user: hostConfig.user,
      keyPath,
      publicKey,
      message: `系统未安装 sshpass，无法自动使用密码下发公钥。请手动追加:\n${publicKey}`
    };
  }
  const sshpass = findBinary('sshpass');
  const script = authorizedKeysScript(publicKey);
  const args = [
    '-e', 'ssh',
    '-o', 'StrictHostKeyChecking=accept-new',
    '-o', 'LogLevel=ERROR',
    '-o', 'PreferredAuthentications=password,keyboard-interactive',
    '-o', 'PubkeyAuthentication=no',
    '-o', 'ConnectTimeout=10',
    '-p', String(hostConfig.port || 22),
    `${hostConfig.user || 'root'}@${hostConfig.host}`,
    'bash -s'
  ];
  const deployed = await run(sshpass, args, {
    env: { SSHPASS: password },
    input: stdinProgram(script),
    timeoutMs: 60000
  });
  if (deployed.code !== 0) {
    return {
      success: false,
      host: hostConfig.host,
      user: hostConfig.user,
      keyPath,
      publicKey,
      message: `自动下发公钥失败: ${explainSshFailure(deployed.stderr, updated)}`
    };
  }
  const verify = await testSshConnection(updated);
  return {
    success: verify.success,
    host: hostConfig.host,
    user: hostConfig.user,
    keyPath,
    publicKey,
    message: verify.success
      ? `公钥已部署并通过免密登录验证。本地私钥: ${keyPath}`
      : verify.message
  };
}

async function uploadStaged(hostConfig, target, kind, input, options) {
  const timeoutMs = options.timeoutMs ?? 30 * 60 * 1000;
  const prepared = await runRemote(hostConfig, transferScript('prepare', target), { timeoutMs: 30000 });
  const stage = prepared.stdout.trim();
  if (!stage.startsWith(`${target}.deploy-stage.`) || /[\r\n]/.test(stage)) {
    throw new DeployError('无法确认暂存目录，已停止上传');
  }
  try {
    const result = await run(requireBinary('ssh'), buildSshArgs(hostConfig, [
      bashCommand(transferScript(`receive-${kind}`, target, stage))
    ]), { ...input, timeoutMs });
    if (result.code !== 0 || result.timedOut || result.stdoutTruncated || result.stderrTruncated) {
      throw new DeployError(result.timedOut ? '上传超时，请检查目标状态后重试' : '上传未完成，原部署未修改', {
        code: 'UPLOAD_FAILED', details: { exitCode: result.code, stderr: result.stderr }
      });
    }
    await runRemote(hostConfig, transferScript('commit', target, stage), { timeoutMs });
    return result;
  } finally {
    // Never mask the original error. Cleanup refuses to remove a retained rollback copy.
    await runRemote(hostConfig, transferScript('cleanup', target, stage), { timeoutMs: 30000 }).catch(() => {});
  }
}

export async function uploadFile(hostConfig, localPath, remotePath, options = {}) {
  const stat = await fsp.stat(localPath);
  if (!stat.isFile()) throw new DeployError('上传来源不是文件，请检查制品内容');
  const stream = fs.createReadStream(localPath);
  let uploaded = 0;
  stream.on('data', (chunk) => {
    uploaded += chunk.length;
    try { options.onProgress?.(uploaded, stat.size); } catch { /* progress cannot corrupt the transfer */ }
  });
  // Pause until the receiver owns the stream; attaching data listeners otherwise starts reading early.
  stream.pause();
  // Prevent an early open error from being unhandled while the staging command runs.
  let sourceError;
  stream.on('error', (error) => { sourceError = error; });
  try {
    if (sourceError) throw sourceError;
    const result = await uploadStaged(hostConfig, remotePath, 'file', { input: stream }, options);
    return { remotePath, sizeBytes: stat.size, durationMs: result.durationMs };
  } finally { stream.destroy(); }
}

export async function uploadDir(hostConfig, localDir, remoteDir, options = {}) {
  const stat = await fsp.stat(localDir);
  if (!stat.isDirectory()) throw new DeployError('上传来源不是目录，请检查制品内容');
  await uploadStaged(hostConfig, remoteDir, 'dir', {
    inputCommand: { command: requireBinary('tar'), args: ['-czf', '-', '-C', localDir, '.'] }
  }, options);
  return { remoteDir };
}

export async function downloadFile(hostConfig, remotePath, localPath, options = {}) {
  const ssh = requireBinary('ssh');
  const result = await run(ssh, buildSshArgs(hostConfig, [`cat ${shellQuote(remotePath)}`]), {
    timeoutMs: options.timeoutMs ?? 30 * 60 * 1000
  });
  if (result.code !== 0) throw new DeployError(explainSshFailure(result.stderr, hostConfig));
  await fsp.writeFile(localPath, result.stdout);
  return { localPath, sizeBytes: Buffer.byteLength(result.stdout) };
}
