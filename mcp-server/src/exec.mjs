import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { DeployError } from './errors.mjs';

const DEFAULT_TIMEOUT_MS = 10 * 60 * 1000;
const DEFAULT_MAX_OUTPUT_BYTES = 4 * 1024 * 1024;

function pathCandidates(name) {
  const pathValue = process.env.PATH || '';
  const dirs = pathValue.split(path.delimiter).filter(Boolean);
  if (process.platform === 'win32') {
    const pathext = (process.env.PATHEXT || '.EXE;.CMD;.BAT;.COM').split(';').filter(Boolean);
    return dirs.flatMap((dir) => [name, ...pathext.map((ext) => `${name}${ext.toLowerCase()}`), ...pathext.map((ext) => `${name}${ext}`)]);
  }
  return dirs.map((dir) => path.join(dir, name));
}

export function findBinary(name) {
  if (!name) return null;
  if (path.isAbsolute(name)) {
    try {
      fs.accessSync(name, fs.constants.X_OK);
      return name;
    } catch {
      return null;
    }
  }
  for (const candidate of pathCandidates(name)) {
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      return candidate;
    } catch {
      // keep searching
    }
  }
  return null;
}

export function requireBinary(name) {
  const binary = findBinary(name);
  if (!binary) {
    throw new DeployError(`未找到命令 ${name}，请确认已安装并加入 PATH`, {
      code: 'BINARY_NOT_FOUND',
      hint: name === 'docker'
        ? '请先安装 Docker，或在远程目标机上执行 docker_install 工具。'
        : `请安装 ${name} 后重试。`
    });
  }
  return binary;
}

function appendLimited(bufferState, chunk, limit) {
  bufferState.total += chunk.length;
  if (bufferState.total <= limit) {
    bufferState.parts.push(chunk);
    return;
  }
  const remaining = limit - (bufferState.total - chunk.length);
  if (remaining > 0) bufferState.parts.push(chunk.subarray(0, remaining));
  bufferState.truncated = true;
}

export function run(command, args = [], options = {}) {
  const {
    cwd,
    env,
    input,
    timeoutMs = DEFAULT_TIMEOUT_MS,
    maxOutputBytes = DEFAULT_MAX_OUTPUT_BYTES,
    onStdout,
    onStderr
  } = options;

  return new Promise((resolve, reject) => {
    const started = Date.now();
    let child;
    try {
      child = spawn(command, args, {
        cwd,
        env: env ? { ...process.env, ...env } : process.env,
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true
      });
    } catch (error) {
      reject(new DeployError(`启动命令失败: ${error.message}`, { code: 'SPAWN_FAILED', details: { command } }));
      return;
    }

    const stdoutState = { parts: [], total: 0, truncated: false };
    const stderrState = { parts: [], total: 0, truncated: false };
    let timedOut = false;
    let settled = false;

    const timer = timeoutMs > 0
      ? setTimeout(() => {
          timedOut = true;
          try {
            child.kill('SIGTERM');
          } catch {
            // ignore
          }
          setTimeout(() => {
            try {
              child.kill('SIGKILL');
            } catch {
              // ignore
            }
          }, 2000).unref?.();
        }, timeoutMs)
      : null;

    child.stdout.on('data', (chunk) => {
      appendLimited(stdoutState, chunk, maxOutputBytes);
      onStdout?.(chunk.toString());
    });
    child.stderr.on('data', (chunk) => {
      appendLimited(stderrState, chunk, maxOutputBytes);
      onStderr?.(chunk.toString());
    });
    child.on('error', (error) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      reject(new DeployError(`执行命令失败: ${error.message}`, { code: 'SPAWN_FAILED', details: { command } }));
    });
    child.on('close', (code, signal) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      resolve({
        command,
        args,
        code: code ?? (timedOut ? 124 : 1),
        signal,
        timedOut,
        stdout: Buffer.concat(stdoutState.parts).toString('utf8'),
        stderr: Buffer.concat(stderrState.parts).toString('utf8'),
        stdoutTruncated: stdoutState.truncated,
        stderrTruncated: stderrState.truncated,
        durationMs: Date.now() - started
      });
    });

    if (input !== undefined && input !== null) {
      child.stdin.end(input);
    } else {
      child.stdin.end();
    }
  });
}

export async function runChecked(command, args = [], options = {}) {
  const result = await run(command, args, options);
  if (result.code !== 0) {
    const stderr = result.stderr.trim();
    const stdout = result.stdout.trim();
    const message = stderr || stdout || (result.timedOut ? '命令执行超时' : `命令退出码 ${result.code}`);
    throw new DeployError(message, {
      code: result.timedOut ? 'COMMAND_TIMEOUT' : 'COMMAND_FAILED',
      details: {
        command: options.displayCommand || path.basename(command),
        exitCode: result.code,
        timedOut: result.timedOut
      }
    });
  }
  return result;
}

export function commandExists(name) {
  return Boolean(findBinary(name));
}
