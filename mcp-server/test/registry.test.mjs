import assert from 'node:assert/strict';
import http from 'node:http';
import test from 'node:test';
import { saveConfig } from '../src/config.mjs';
import { parseWwwAuthenticate, probeRegistry, registryFromArgs } from '../src/registry.mjs';
import { makeTempDir } from './helpers.mjs';

const ORIGINAL_configDir = process.env.DEPLOY_DESKTOP_CONFIG_DIR;
const ORIGINAL_configFile = process.env.DEPLOY_DESKTOP_CONFIG_FILE;

function restoreEnv() {
  if (ORIGINAL_configDir === undefined) delete process.env.DEPLOY_DESKTOP_CONFIG_DIR;
  else process.env.DEPLOY_DESKTOP_CONFIG_DIR = ORIGINAL_configDir;
  if (ORIGINAL_configFile === undefined) delete process.env.DEPLOY_DESKTOP_CONFIG_FILE;
  else process.env.DEPLOY_DESKTOP_CONFIG_FILE = ORIGINAL_configFile;
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

test('parseWwwAuthenticate 解析 Bearer 参数', () => {
  const challenge = parseWwwAuthenticate(
    'Bearer realm="https://auth.example.com/token",service="registry.example.com",scope="repository:team/backend:pull"'
  );
  assert.equal(challenge.scheme, 'bearer');
  assert.equal(challenge.params.realm, 'https://auth.example.com/token');
  assert.equal(challenge.params.service, 'registry.example.com');
  assert.equal(challenge.params.scope, 'repository:team/backend:pull');
  assert.equal(challenge.raw.startsWith('Bearer'), true);
});

test('parseWwwAuthenticate 解析 Basic 并处理非法输入', () => {
  const challenge = parseWwwAuthenticate('Basic realm="Registry"');
  assert.equal(challenge.scheme, 'basic');
  assert.equal(challenge.params.realm, 'Registry');
  assert.equal(parseWwwAuthenticate(''), null);
  assert.equal(parseWwwAuthenticate('not-a-challenge'), null);
});

test('registryFromArgs 解析保存的仓库配置与密码环境变量', async () => {
  process.env.DEPLOY_DESKTOP_CONFIG_DIR = await makeTempDir();
  delete process.env.DEPLOY_DESKTOP_CONFIG_FILE;
  process.env.TEST_ACR_PASSWORD = 's3cret';
  try {
    await saveConfig({
      registries: {
        acr: {
          name: 'acr',
          url: 'registry.example.com',
          namespace: 'team',
          username: 'robot',
          passwordEnv: 'TEST_ACR_PASSWORD',
          insecure: false,
          isDefault: true
        }
      }
    });
    const registry = await registryFromArgs({ registry: 'acr' });
    assert.equal(registry.url, 'registry.example.com');
    assert.equal(registry.namespace, 'team');
    assert.equal(registry.username, 'robot');
    assert.equal(registry.password, 's3cret');

    const defaultRegistry = await registryFromArgs({});
    assert.equal(defaultRegistry.url, 'registry.example.com');
  } finally {
    delete process.env.TEST_ACR_PASSWORD;
    restoreEnv();
  }
});

test('probeRegistry 探测本地 Registry V2 接口', async () => {
  const server = http.createServer((request, response) => {
    if (request.url === '/v2/') {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end('{}');
      return;
    }
    response.writeHead(404);
    response.end();
  });
  const port = await listen(server);
  try {
    const result = await probeRegistry({ url: `127.0.0.1:${port}`, insecure: true, username: '' });
    assert.equal(result.ok, true);
    assert.equal(result.status, 200);
    assert.equal(result.registry, `127.0.0.1:${port}`);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});
