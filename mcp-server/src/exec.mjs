import { spawn } from 'node:child_process';
import fs from 'node:fs';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
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

// One lifecycle for commands, file streams and producer -> consumer pipelines.
export async function run(command, args = [], options = {}) {
  const { cwd, env, input, inputCommand, timeoutMs = DEFAULT_TIMEOUT_MS,
    maxOutputBytes = DEFAULT_MAX_OUTPUT_BYTES, onStdout, onStderr } = options;
  const started = Date.now();
  const children = [];
  let timedOut = false;
  let callbackError;
  let killTimer;
  const kill = (signal) => {
    for (const child of children) {
      try {
        if (process.platform !== 'win32' && child.pid) process.kill(-child.pid, signal);
        else child.kill(signal);
      } catch { /* already exited */ }
    }
  };
  const start = (program, argv, pipeOutput = false) => {
    const child = spawn(program, argv, {
      cwd, env: env ? { ...process.env, ...env } : process.env,
      stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
      detached: process.platform !== 'win32'
    });
    children.push(child);
    const stdout = { parts: [], total: 0, truncated: false };
    const stderr = { parts: [], total: 0, truncated: false };
    const collect = (state, callback) => (chunk) => {
      appendLimited(state, chunk, maxOutputBytes);
      try { callback?.(chunk.toString()); }
      catch (error) { callbackError = error; kill('SIGKILL'); }
    };
    if (!pipeOutput) child.stdout.on('data', collect(stdout, onStdout));
    child.stderr.on('data', collect(stderr, pipeOutput ? undefined : onStderr));
    // Every pipe has an error listener before input is attached (early SSH exits).
    child.stdin.on('error', () => {});
    let spawnError;
    child.on('error', (error) => { spawnError = error; kill('SIGKILL'); });
    const done = new Promise((resolve) => child.once('close', (code, signal) => resolve({
      code: code ?? 1, signal, spawnError,
      stdout: Buffer.concat(stdout.parts).toString('utf8'),
      stderr: Buffer.concat(stderr.parts).toString('utf8'),
      stdoutTruncated: stdout.truncated, stderrTruncated: stderr.truncated
    })));
    return { child, done };
  };
  let receiver;
  let producer;
  let timer;
  try {
    receiver = start(command, args);
    if (inputCommand) producer = start(inputCommand.command, inputCommand.args, true);
    timer = timeoutMs > 0 ? setTimeout(() => {
      timedOut = true;
      kill('SIGTERM');
      killTimer = setTimeout(() => {
        kill('SIGKILL');
        // Inherited handles must not keep close/pipeline pending after the deadline.
        for (const child of children) {
          child.stdin.destroy(); child.stdout.destroy(); child.stderr.destroy();
        }
        if (input?.destroy) input.destroy();
      }, 2000);
    }, timeoutMs) : null;
    if (producer) producer.child.stdin.end();
    const source = producer?.child.stdout
      || (input?.pipe ? input : Readable.from(input == null ? [] : [input]));
    // Handle rejection immediately and wait for both exit statuses before committing.
    const feed = pipeline(source, receiver.child.stdin).then(() => null, (error) => error);
    const [result, inputError, sourceResult] = await Promise.all([
      receiver.done, feed, producer?.done || Promise.resolve(null)
    ]);
    if (timedOut) return { ...result, command, args, code: 124, timedOut: true, durationMs: Date.now() - started };
    const spawnError = result.spawnError || sourceResult?.spawnError;
    if (spawnError) throw new DeployError(`启动执行进程失败: ${spawnError.message}`, { code: 'SPAWN_FAILED' });
    if (callbackError) throw new DeployError(`执行进度处理失败: ${callbackError.message}`, { code: 'CALLBACK_FAILED' });
    if (sourceResult && sourceResult.code !== 0) {
      throw new DeployError(`打包失败: ${sourceResult.stderr || `退出码 ${sourceResult.code}`}`, { code: 'PRODUCER_FAILED' });
    }
    if (inputError && result.code === 0) throw new DeployError(`传输输入失败: ${inputError.message}`, { code: 'INPUT_FAILED' });
    if (sourceResult?.stderrTruncated) throw new DeployError('打包输出超过限制，请检查本地文件', { code: 'OUTPUT_LIMIT' });
    return { ...result, command, args, timedOut: false, durationMs: Date.now() - started };
  } finally {
    clearTimeout(timer);
    clearTimeout(killTimer);
    // Reap any still-running descendants, including on synchronous setup failure.
    kill('SIGKILL');
    await Promise.all(children.map((child) => child.exitCode !== null || child.signalCode !== null
      ? Promise.resolve() : new Promise((resolve) => child.once('close', resolve))));
  }
}

export async function runChecked(command, args = [], options = {}) {
  const result = await run(command, args, options);
  if (result.code !== 0 || result.timedOut || result.stdoutTruncated || result.stderrTruncated) {
    const stderr = result.stderr.trim();
    const stdout = result.stdout.trim();
    const message = result.timedOut ? '命令执行超时' : (result.stdoutTruncated || result.stderrTruncated) ? '执行输出超过限制，请检查目标状态与日志' : stderr || stdout || `命令退出码 ${result.code}`;
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
