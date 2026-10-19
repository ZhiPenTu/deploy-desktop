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
import { makeProfile, makeTempDir } from './helpers.mjs';

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

test('normalizeComponents 去重并拒绝未知组件', () => {
  assert.deepEqual(normalizeComponents(['backend', 'BACKEND', 'gateway']), ['backend', 'gateway']);
  assert.throws(() => normalizeComponents(['database']), /不支持的更新组件/);
  assert.throws(() => normalizeComponents([]), /至少选择一个更新组件/);
});

test('resolveUpdateEndpoint 默认直接使用 GitHub Releases', () => {
  assert.equal(resolveUpdateEndpoint(), DEFAULT_UPDATE_ENDPOINT);
  assert.equal(resolveUpdateEndpoint('   '), DEFAULT_UPDATE_ENDPOINT);
  assert.equal(resolveUpdateEndpoint(' https://example.com/latest.json '), 'https://example.com/latest.json');
});

test('validateBundle 识别完整更新包', async () => {
  const root = await makeTempDir();
  const profile = makeProfile();
  await fs.mkdir(path.join(root, 'app', 'gateway-classes'), { recursive: true });
  await fs.mkdir(path.join(root, 'app', 'gateway-lib'), { recursive: true });
  await fs.mkdir(path.join(root, 'app', 'frontend-dist'), { recursive: true });
  await fs.writeFile(path.join(root, 'app', profile.backendArtifact), 'jar');
  await fs.writeFile(path.join(root, 'app', 'frontend-dist', 'index.html'), '<html></html>');
  await fs.mkdir(path.join(root, 'deploy', 'nginx'), { recursive: true });
  await fs.mkdir(path.join(root, 'deploy', 'oceanbase'), { recursive: true });
  await fs.writeFile(path.join(root, 'deploy', 'docker-compose.yml'), 'services: {}\n');

  const paths = await validateBundle(profile, root, ['backend', 'gateway', 'frontend', 'config']);
  assert.equal(paths.serverJar, path.join(root, 'app', profile.backendArtifact));
  assert.equal(paths.gatewayClasses, path.join(root, 'app', 'gateway-classes'));
  assert.equal(paths.gatewayLib, path.join(root, 'app', 'gateway-lib'));
  assert.equal(paths.frontendDist, path.join(root, 'app', 'frontend-dist'));
  assert.equal(paths.composeFile, path.join(root, 'deploy', 'docker-compose.yml'));
  assert.equal(paths.nginxDir, path.join(root, 'deploy', 'nginx'));
  assert.equal(paths.oceanbaseDir, path.join(root, 'deploy', 'oceanbase'));
});

test('validateBundle 报告缺失文件', async () => {
  const root = await makeTempDir();
  await assert.rejects(
    validateBundle(makeProfile(), root, ['backend', 'frontend']),
    (error) => error.code === 'BUNDLE_INCOMPLETE' && /app\/demo-server\.jar/.test(error.message)
  );
});

test('validateBundle 拒绝不存在的目录', async () => {
  await assert.rejects(validateBundle(makeProfile(), '/definitely/not/here', ['backend']), /更新包目录不存在/);
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
