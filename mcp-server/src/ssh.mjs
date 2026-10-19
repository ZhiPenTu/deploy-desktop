import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { pipeline } from 'node:stream/promises';
import { loadConfig, validateHost } from './config.mjs';
import { DeployError } from './errors.mjs';
import { commandExists, findBinary, requireBinary, run } from './exec.mjs';
import { loadProfile } from './profile.mjs';
import { expandHome, shellQuote } from './util.mjs';

const DEFAULT_KEY_CANDIDATES = ['id_ed25519', 'id_rsa', 'id_ecdsa'];

function isLikelyHostAddress(value) {
  return /^[A-Za-z0-9._:[\]-]+$/.test(value);
}

export async function resolveHost(input, overrides = {}) {
  const { config } = await loadConfig();
  const key = String(input || '').trim();
  if (!key) throw new DeployError('缺少 host 参数');
  const configured = config.hosts[key];
  const { profile } = await loadProfile();
  const base = configured
    ? { ...configured }
    : {
        name: isLikelyHostAddress(key) ? key : '',
        host: isLikelyHostAddress(key) ? key : '',
        port: 22,
        user: 'root'
      };
  if (!base.host) {
    throw new DeployError(`未找到主机配置: ${key}`, {
      hint: '请先使用 deploy_host_save 保存主机，或直接传入 IP/域名。'
    });
  }
  for (const field of ['port', 'user', 'keyPath', 'installDir', 'deployDir', 'bundleDir', 'strictHostKeyChecking']) {
    if (overrides[field] !== undefined && overrides[field] !== null && String(overrides[field]).trim() !== '') {
      base[field] = overrides[field];
    }
  }
  base.installDir = expandHome(base.installDir || profile.installDir);
  base.deployDir = expandHome(base.deployDir || `${base.installDir}/deploy`);
  base.bundleDir = expandHome(base.bundleDir || profile.bundleDir);
  base.keyPath = base.keyPath ? expandHome(base.keyPath) : '';
  return { host: validateHost(base), configured: Boolean(configured), profile };
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
  args.push(...extra, `${hostConfig.user || 'root'}@${hostConfig.host}`);
  return args;
}

export async function runRemote(hostConfig, script, options = {}) {
  const ssh = requireBinary('ssh');
  const result = await run(ssh, buildSshArgs(hostConfig, ['bash -s']), {
    input: script,
    timeoutMs: options.timeoutMs ?? 10 * 60 * 1000,
    maxOutputBytes: options.maxOutputBytes,
    onStdout: options.onStdout,
    onStderr: options.onStderr
  });
  if (result.code !== 0 && options.throwOnError !== false) {
    throw new DeployError(explainSshFailure(result.stderr || result.stdout, hostConfig), {
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
  } else if (lower.includes('no such file') || lower.includes('not found')) {
    hint = '系统未找到 ssh 命令，请确认 OpenSSH 客户端已安装。';
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
    input: script,
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

async function runStreaming(command, args, feed, options = {}) {
  return new Promise((resolve, reject) => {
    const started = Date.now();
    const child = spawn(command, args, {
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true
    });
    const stderrParts = [];
    let stderrBytes = 0;
    let settled = false;
    const timer = options.timeoutMs
      ? setTimeout(() => {
          try { child.kill('SIGTERM'); } catch { /* ignore */ }
        }, options.timeoutMs)
      : null;
    child.stderr.on('data', (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes <= 1024 * 1024) stderrParts.push(chunk);
    });
    child.on('error', (error) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      reject(new DeployError(`启动命令失败: ${error.message}`, { code: 'SPAWN_FAILED' }));
    });
    child.on('close', (code, signal) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      resolve({
        code: code ?? 1,
        signal,
        stderr: Buffer.concat(stderrParts).toString('utf8'),
        durationMs: Date.now() - started,
        timedOut: false
      });
    });
    Promise.resolve()
      .then(() => feed(child.stdin))
      .catch((error) => {
        try { child.kill('SIGKILL'); } catch { /* ignore */ }
        reject(new DeployError(`上传数据流失败: ${error.message}`));
      });
  });
}

export async function uploadFile(hostConfig, localPath, remotePath, options = {}) {
  const stat = await fsp.stat(localPath);
  const ssh = requireBinary('ssh');
  const temp = `${remotePath}.tt-new`;
  const script = `set -e
rm -f ${shellQuote(temp)}
cat > ${shellQuote(temp)}
mv -f ${shellQuote(temp)} ${shellQuote(remotePath)}
`;
  let uploaded = 0;
  const feed = async (stdin) => {
    const stream = fs.createReadStream(localPath);
    stream.on('data', (chunk) => {
      uploaded += chunk.length;
      options.onProgress?.(uploaded, stat.size);
    });
    await pipeline(stream, stdin);
  };
  const result = await runStreaming(ssh, buildSshArgs(hostConfig, ['bash -s']), feed, {
    timeoutMs: options.timeoutMs ?? 30 * 60 * 1000
  });
  if (result.code !== 0) {
    throw new DeployError(explainSshFailure(result.stderr, hostConfig), {
      code: 'UPLOAD_FAILED',
      details: { localPath, remotePath }
    });
  }
  return { remotePath, sizeBytes: stat.size, durationMs: result.durationMs };
}

export async function uploadDir(hostConfig, localDir, remoteDir, options = {}) {
  const tar = requireBinary('tar');
  const ssh = requireBinary('ssh');
  const temp = `${remoteDir}.tt-new`;
  const script = `set -e
rm -rf ${shellQuote(temp)}
mkdir -p ${shellQuote(temp)}
tar -xzf - -C ${shellQuote(temp)}
rm -rf ${shellQuote(remoteDir)}
mv ${shellQuote(temp)} ${shellQuote(remoteDir)}
`;
  const tarProcess = spawn(tar, ['-czf', '-', '-C', localDir, '.'], {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true
  });
  const tarErrors = [];
  tarProcess.stderr.on('data', (chunk) => tarErrors.push(chunk));
  const sshProcess = spawn(ssh, buildSshArgs(hostConfig, ['bash -s']), {
    stdio: ['pipe', 'pipe', 'pipe'],
    windowsHide: true
  });
  const sshErrors = [];
  sshProcess.stderr.on('data', (chunk) => sshErrors.push(chunk));
  const tarDone = new Promise((resolve, reject) => {
    tarProcess.on('error', reject);
    tarProcess.on('close', (code) => (code === 0 ? resolve() : reject(new Error(Buffer.concat(tarErrors).toString('utf8') || `tar 退出码 ${code}`))));
  });
  const sshDone = new Promise((resolve, reject) => {
    sshProcess.on('error', reject);
    sshProcess.on('close', (code) => resolve(code));
  });
  try {
    await pipeline(tarProcess.stdout, sshProcess.stdin);
    await tarDone;
    const code = await sshDone;
    if (code !== 0) {
      throw new DeployError(explainSshFailure(Buffer.concat(sshErrors).toString('utf8'), hostConfig), {
        code: 'UPLOAD_FAILED',
        details: { localDir, remoteDir }
      });
    }
  } catch (error) {
    try { sshProcess.kill('SIGKILL'); } catch { /* ignore */ }
    try { tarProcess.kill('SIGKILL'); } catch { /* ignore */ }
    if (error instanceof DeployError) throw error;
    throw new DeployError(`上传目录失败: ${error.message}`, { details: { localDir, remoteDir } });
  }
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
