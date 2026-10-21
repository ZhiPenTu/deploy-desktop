import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import test from 'node:test';
import {
  checkAppUpdate,
  DEFAULT_UPDATE_ENDPOINT,
  normalizeComponents,
  resolveUpdateEndpoint,
  validateBundle
} from '../src/update.mjs';
import { makeTempDir, writeBundle } from './helpers.mjs';
import { Bundle } from '../src/bundle.mjs';

function expectedPlatformKey() {
  if (process.platform === 'darwin') return process.arch === 'arm64' ? 'darwin-aarch64' : 'darwin-x86_64';
  if (process.platform === 'win32') return 'windows-x86_64';
  if (process.platform === 'linux') return process.arch === 'arm64' ? 'linux-aarch64' : 'linux-x86_64';
  return `${process.platform}-${process.arch}`;
}

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject);
      resolve(server.address().port);
    });
  });
}

test('normalizeComponents 去重并拒绝未知组件', async () => {
  const root = await makeTempDir();
  await writeBundle(root);
  const bundle = await Bundle.load(root);
  assert.deepEqual(normalizeComponents(bundle, ['server', 'site', 'server']), ['server', 'site']);
  assert.throws(() => normalizeComponents(bundle, ['database']), /未声明组件 database/);
  assert.throws(() => normalizeComponents(bundle, []), /至少选择一个更新组件/);
});

test('resolveUpdateEndpoint 默认直接使用 GitHub Releases', () => {
  assert.equal(resolveUpdateEndpoint(), DEFAULT_UPDATE_ENDPOINT);
  assert.equal(resolveUpdateEndpoint('   '), DEFAULT_UPDATE_ENDPOINT);
  assert.equal(resolveUpdateEndpoint(' https://example.com/latest.json '), 'https://example.com/latest.json');
});

test('validateBundle 按交付包解析产物与部署路径', async () => {
  const root = await makeTempDir();
  await writeBundle(root);
  const profile = { product: 'demo-product', installDir: '/opt/demo', deployDir: '/opt/demo/deploy' };
  const { artifacts } = await validateBundle(profile, root, ['server', 'site', 'config']);
  const byKey = Object.fromEntries(artifacts.map((item) => [item.key, item]));
  assert.equal(byKey['server-bin'].target, '/opt/demo/bin');
  assert.equal(byKey['server-config'].target, '/opt/demo/config/app.yml');
  assert.equal(byKey['web-dist'].target, '/opt/demo/deploy/site');
  assert.equal(byKey['compose'].target, '/opt/demo/deploy/docker-compose.yml');
  assert.deepEqual(artifacts.map((item) => item.key), ['server-bin', 'server-config', 'web-dist', 'compose', 'db-config']);
});

test('validateBundle 拒绝摘要不符的交付包', async () => {
  const root = await makeTempDir();
  await writeBundle(root);
  await fs.writeFile(path.join(root, 'app/config/app.yml'), 'tampered\n');
  const profile = { product: 'demo-product', installDir: '/opt/demo', deployDir: '/opt/demo/deploy' };
  await assert.rejects(validateBundle(profile, root, ['server']), (error) => error.code === 'BUNDLE_SIZE_MISMATCH' || error.code === 'BUNDLE_DIGEST_MISMATCH');
});

test('validateBundle 拒绝不匹配的产品与不存在的目录', async () => {
  const root = await makeTempDir();
  await writeBundle(root);
  const profile = { product: 'other-product', installDir: '/opt/demo', deployDir: '/opt/demo/deploy' };
  await assert.rejects(validateBundle(profile, root, ['server']), (error) => error.code === 'PRODUCT_MISMATCH');
  await assert.rejects(validateBundle({ ...profile, product: 'demo-product' }, '/definitely/not/here', ['server']), /更新包目录不存在/);
});

test('checkAppUpdate 比较版本并返回当前平台下载信息', async () => {
  const key = expectedPlatformKey();
  const server = http.createServer((request, response) => {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({
      version: '9.9.9',
      notes: 'test release',
      pub_date: '2026-10-19T00:00:00Z',
      platforms: {
        [key]: { url: 'https://example.com/app.tar.gz', signature: 'sig' }
      }
    }));
  });
  const port = await listen(server);
  try {
    const result = await checkAppUpdate({
      endpoint: `http://127.0.0.1:${port}/latest.json`,
      allowInsecure: true,
      currentVersion: '0.1.0'
    });
    assert.equal(result.available, true);
    assert.equal(result.latestVersion, '9.9.9');
    assert.equal(result.platform, key);
    assert.equal(result.url, 'https://example.com/app.tar.gz');
    assert.equal(result.signature, 'present');
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});

test('checkAppUpdate 对相同版本返回无需更新', async () => {
  const server = http.createServer((request, response) => {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ version: '1.0.0', platforms: {} }));
  });
  const port = await listen(server);
  try {
    const result = await checkAppUpdate({
      endpoint: `http://127.0.0.1:${port}/latest.json`,
      allowInsecure: true,
      currentVersion: '1.0.0'
    });
    assert.equal(result.available, false);
    assert.match(result.message, /已是最新版本/);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});

test('checkAppUpdate 默认拒绝 HTTP 更新源', async () => {
  await assert.rejects(
    checkAppUpdate({ endpoint: 'http://127.0.0.1:1/latest.json', currentVersion: '1.0.0' }),
    /必须使用 HTTPS/
  );
});
