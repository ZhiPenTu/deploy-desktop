import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { lockPath, withFileLock } from '../src/file-lock.mjs';
import { mutateProjectStore, PROJECTS_FILE } from '../src/projects.mjs';
import { makeTempDir } from './helpers.mjs';

const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const ORIGINAL_APP_CONFIG_DIR = process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;

async function withAppConfigDir(fn) {
  const dir = await makeTempDir('deploy-mcp-lock-');
  process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = dir;
  try {
    return await fn(dir);
  } finally {
    if (ORIGINAL_APP_CONFIG_DIR === undefined) delete process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
    else process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = ORIGINAL_APP_CONFIG_DIR;
    await fs.rm(dir, { recursive: true, force: true });
  }
}

test('withFileLock 结束后释放锁文件', async () => {
  const dir = await makeTempDir('deploy-mcp-lock-');
  const target = path.join(dir, 'sample.json');
  try {
    const value = await withFileLock(target, async () => {
      assert.ok((await fs.stat(lockPath(target))).isFile(), '执行期间应持有锁');
      return 'done';
    });
    assert.equal(value, 'done');
    await assert.rejects(fs.stat(lockPath(target)), { code: 'ENOENT' });
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test('并发调用按顺序串行执行', async () => {
  const dir = await makeTempDir('deploy-mcp-lock-');
  const target = path.join(dir, 'sample.json');
  let active = 0;
  let peak = 0;
  try {
    await Promise.all(
      Array.from({ length: 5 }, () =>
        withFileLock(target, async () => {
          active += 1;
          peak = Math.max(peak, active);
          await delay(10);
          active -= 1;
        })
      )
    );
    assert.equal(peak, 1, '同一时刻只应有一个写入方');
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test('超时后返回可重试提示', async () => {
  const dir = await makeTempDir('deploy-mcp-lock-');
  const target = path.join(dir, 'sample.json');
  try {
    await fs.writeFile(lockPath(target), '1\n');
    await assert.rejects(withFileLock(target, async () => 'never', { timeoutMs: 60 }), (error) => {
      assert.equal(error.code, 'CONFIG_LOCKED');
      assert.match(error.message, /稍后重试/);
      return true;
    });
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test('进程异常留下的残留锁会被回收', async () => {
  const dir = await makeTempDir('deploy-mcp-lock-');
  const target = path.join(dir, 'sample.json');
  try {
    await fs.writeFile(lockPath(target), '1\n');
    const old = new Date(Date.now() - 120_000);
    await fs.utimes(lockPath(target), old, old);
    assert.equal(await withFileLock(target, async () => 'reclaimed'), 'reclaimed');
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

test('mutateProjectStore 在锁内读取其他写入方刚保存的项目', async () => {
  await withAppConfigDir(async (dir) => {
    const file = path.join(dir, PROJECTS_FILE);
    // 模拟桌面端正在写：先占住锁，再在持锁期间写入磁盘。
    await fs.writeFile(lockPath(file), '1\n');

    let finished = false;
    const pending = mutateProjectStore((store) => {
      store.projects.push({
        id: 'project-local',
        name: '本机新增',
        product: '',
        hostId: 'host-1',
        installDir: '/opt/app',
        sourceId: '',
        lastVersion: '',
        lastBundleDir: ''
      });
    });
    pending.then(() => {
      finished = true;
    });

    await delay(80);
    assert.equal(finished, false, '写入方持锁期间应等待而不是直接覆盖');

    await fs.writeFile(
      file,
      JSON.stringify({
        schemaVersion: 1,
        activeProjectId: 'project-desktop',
        projects: [
          {
            id: 'project-desktop',
            name: '桌面端新增',
            product: '',
            hostId: 'host-1',
            installDir: '/opt/app',
            sourceId: '',
            lastVersion: '',
            lastBundleDir: ''
          }
        ]
      })
    );
    await fs.rm(lockPath(file), { force: true });

    const { store } = await pending;
    assert.deepEqual(store.projects.map((project) => project.id).sort(), [
      'project-desktop',
      'project-local'
    ]);
    assert.equal(store.activeProjectId, 'project-desktop');
    await assert.rejects(fs.stat(lockPath(file)), { code: 'ENOENT' });
  });
});
