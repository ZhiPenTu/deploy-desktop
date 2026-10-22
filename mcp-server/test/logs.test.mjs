import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import path from 'node:path';
import { makeTempDir } from './helpers.mjs';
import {
  parseLogLine,
  readRuntimeLogs,
  runtimeLogDir,
  runtimeLogSummary
} from '../src/logs.mjs';

const BACKUP = [
  '2026-10-21 09:00:00.000 INFO app - 应用启动，版本 0.3.3',
  '2026-10-21 09:01:02.500 WARN bundle - bundle.json 解析失败: unknown field `schemaVersion`',
  '2026-10-21 09:02:03.000 ERROR artifact - 交付包校验或解包失败：版本=9'
].join('\n');

const CURRENT = [
  '2026-10-22 10:00:00.000 INFO artifact - 开始下载交付包：版本=13-79adcb33',
  '2026-10-22 10:00:42.100 INFO artifact - 下载完成：MD5=d54f7c9d',
  '2026-10-22 10:00:43.000 ERROR deploy - 本机部署更新失败：目录=/tmp/demo'
].join('\n');

async function withLogDir(handler) {
  const dir = await makeTempDir('deploy-mcp-logs-');
  await fs.writeFile(path.join(dir, 'runtime.1.log'), `${BACKUP}\n`);
  await fs.writeFile(path.join(dir, 'runtime.log'), `${CURRENT}\n`);
  const previous = process.env.DEPLOY_DESKTOP_LOG_DIR;
  process.env.DEPLOY_DESKTOP_LOG_DIR = dir;
  try {
    return await handler(dir);
  } finally {
    if (previous === undefined) delete process.env.DEPLOY_DESKTOP_LOG_DIR;
    else process.env.DEPLOY_DESKTOP_LOG_DIR = previous;
    await fs.rm(dir, { recursive: true, force: true });
  }
}

test('parseLogLine 解析时间、级别、来源与内容', () => {
  const entry = parseLogLine('2026-10-22 10:00:43.000 ERROR deploy - 部署失败');
  assert.equal(entry.date, '2026-10-22');
  assert.equal(entry.time, '2026-10-22 10:00:43.000');
  assert.equal(entry.level, 'ERROR');
  assert.equal(entry.target, 'deploy');
  assert.equal(entry.message, '部署失败');
});

test('parseLogLine 对无法解析的行回退为应用日志', () => {
  const entry = parseLogLine('一行自由文本');
  assert.equal(entry.level, 'INFO');
  assert.equal(entry.target, 'app');
  assert.equal(entry.message, '一行自由文本');
});

test('runtimeLogDir 支持环境变量覆盖', async () => {
  await withLogDir(async (dir) => {
    assert.equal(await runtimeLogDir(), dir);
  });
});

test('runtimeLogDir 读取应用里设置的日志目录', async () => {
  const configDir = await makeTempDir('deploy-mcp-config-');
  const customDir = await makeTempDir('deploy-mcp-custom-logs-');
  const previousLogDir = process.env.DEPLOY_DESKTOP_LOG_DIR;
  const previousConfigDir = process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
  delete process.env.DEPLOY_DESKTOP_LOG_DIR;
  process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = configDir;
  try {
    await fs.writeFile(
      path.join(configDir, 'log-settings.json'),
      JSON.stringify({ dir: customDir })
    );
    assert.equal(await runtimeLogDir(), customDir);
  } finally {
    if (previousLogDir !== undefined) process.env.DEPLOY_DESKTOP_LOG_DIR = previousLogDir;
    if (previousConfigDir === undefined) delete process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
    else process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = previousConfigDir;
    await fs.rm(configDir, { recursive: true, force: true });
    await fs.rm(customDir, { recursive: true, force: true });
  }
});

test('runtimeLogDir 在设置无效时回落到默认目录', async () => {
  const configDir = await makeTempDir('deploy-mcp-config-');
  const previousLogDir = process.env.DEPLOY_DESKTOP_LOG_DIR;
  const previousConfigDir = process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
  delete process.env.DEPLOY_DESKTOP_LOG_DIR;
  process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = configDir;
  try {
    await fs.writeFile(path.join(configDir, 'log-settings.json'), JSON.stringify({ dir: 'relative/dir' }));
    const dir = await runtimeLogDir();
    assert.ok(path.isAbsolute(dir));
    assert.notEqual(dir, 'relative/dir');
  } finally {
    if (previousLogDir !== undefined) process.env.DEPLOY_DESKTOP_LOG_DIR = previousLogDir;
    if (previousConfigDir === undefined) delete process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
    else process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = previousConfigDir;
    await fs.rm(configDir, { recursive: true, force: true });
  }
});

test('readRuntimeLogs 合并轮转文件并按最新在前返回', async () => {
  await withLogDir(async () => {
    const snapshot = await readRuntimeLogs({});
    assert.equal(snapshot.counts.info, 3);
    assert.equal(snapshot.counts.warn, 1);
    assert.equal(snapshot.counts.error, 2);
    assert.equal(snapshot.entries[0].time, '2026-10-22 10:00:43.000');
    assert.equal(snapshot.entries.at(-1).time, '2026-10-21 09:00:00.000');
  });
});

test('readRuntimeLogs 支持级别与关键词过滤', async () => {
  await withLogDir(async () => {
    const errors = await readRuntimeLogs({ level: 'error' });
    assert.equal(errors.matched, 2);
    assert.ok(errors.entries.every((entry) => entry.level === 'ERROR'));

    const deploy = await readRuntimeLogs({ keyword: 'deploy' });
    assert.equal(deploy.matched, 1);
    assert.match(deploy.entries[0].message, /本机部署更新失败/);
  });
});

test('readRuntimeLogs 限制返回条数并标记截断', async () => {
  await withLogDir(async () => {
    const snapshot = await readRuntimeLogs({ limit: 2 });
    assert.equal(snapshot.returned, 2);
    assert.equal(snapshot.matched, 6);
    assert.equal(snapshot.truncated, true);
  });
});

test('runtimeLogSummary 汇总占用与最近错误', async () => {
  await withLogDir(async () => {
    const summary = await runtimeLogSummary();
    assert.equal(summary.totalCount, 6);
    assert.equal(summary.files.length, 2);
    assert.ok(summary.totalBytes > 0);
    assert.equal(summary.latestError.level, 'ERROR');
    assert.match(summary.latestError.message, /本机部署更新失败/);
    assert.equal(summary.latestWarning.target, 'bundle');
  });
});

test('缺少日志文件时给出明确错误', async () => {
  const dir = await makeTempDir('deploy-mcp-logs-empty-');
  const previous = process.env.DEPLOY_DESKTOP_LOG_DIR;
  process.env.DEPLOY_DESKTOP_LOG_DIR = dir;
  try {
    await assert.rejects(() => readRuntimeLogs({}), /尚未找到本机运行日志/);
  } finally {
    if (previous === undefined) delete process.env.DEPLOY_DESKTOP_LOG_DIR;
    else process.env.DEPLOY_DESKTOP_LOG_DIR = previous;
    await fs.rm(dir, { recursive: true, force: true });
  }
});
