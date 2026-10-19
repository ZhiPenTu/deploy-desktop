import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import {
  configPath,
  ensureDefaultRegistry,
  fileModeIsPrivate,
  loadConfig,
  mutateConfig,
  saveConfig,
  validateHost,
  validateRegistry
} from '../src/config.mjs';
import { makeTempDir } from './helpers.mjs';

const ORIGINAL_ENV = {
  DEPLOY_DESKTOP_CONFIG_DIR: process.env.DEPLOY_DESKTOP_CONFIG_DIR,
  DEPLOY_DESKTOP_CONFIG_FILE: process.env.DEPLOY_DESKTOP_CONFIG_FILE
};

async function useTempConfigDir() {
  const dir = await makeTempDir();
  process.env.DEPLOY_DESKTOP_CONFIG_DIR = dir;
  delete process.env.DEPLOY_DESKTOP_CONFIG_FILE;
  return dir;
}

function restoreEnv() {
  for (const [key, value] of Object.entries(ORIGINAL_ENV)) {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
}

test('validateRegistry 规范化地址并校验字段', () => {
  const registry = validateRegistry({
    name: 'acr',
    url: 'https://Registry.Example.com/',
    namespace: '/team/',
    username: ' robot ',
    passwordEnv: 'ACR_PASSWORD',
    insecure: 1,
    isDefault: 'yes'
  });
  assert.equal(registry.url, 'registry.example.com');
  assert.equal(registry.namespace, 'team');
  assert.equal(registry.username, 'robot');
  assert.equal(registry.insecure, true);
  assert.equal(registry.isDefault, true);

  assert.throws(() => validateRegistry({ url: '' }), /url 不能为空/);
  assert.throws(() => validateRegistry({ url: 'https://registry.example.com/team' }), /仓库地址无效/);
  assert.throws(() => validateRegistry({ url: 'registry example.com' }), /仓库地址无效/);
  assert.throws(() => validateRegistry({ url: 'registry.example.com', namespace: 'bad name' }), /命名空间无效/);
  assert.throws(() => validateRegistry({ url: 'registry.example.com', passwordEnv: '1BAD' }), /passwordEnv/);
});

test('validateHost 补默认值并拒绝非法输入', () => {
  const host = validateHost({ name: 'lan', host: '192.168.1.10' });
  assert.equal(host.port, 22);
  assert.equal(host.user, 'root');
  assert.equal(host.strictHostKeyChecking, 'accept-new');

  assert.throws(() => validateHost({ host: '192.168.1.10', port: 70000 }), /SSH 端口无效/);
  assert.throws(() => validateHost({ host: 'bad host' }), /主机地址含非法字符/);
  assert.throws(() => validateHost({ host: '192.168.1.10', strictHostKeyChecking: 'maybe' }), /strictHostKeyChecking 无效/);
});

test('ensureDefaultRegistry 保证只有一个默认仓库', () => {
  const config = {
    registries: {
      a: { name: 'a', isDefault: true },
      b: { name: 'b', isDefault: true }
    }
  };
  ensureDefaultRegistry(config, 'b');
  assert.equal(config.registries.a.isDefault, false);
  assert.equal(config.registries.b.isDefault, true);
});

test('saveConfig 使用私有权限并支持读取与变更', async () => {
  const dir = await useTempConfigDir();
  try {
    const file = await saveConfig({
      registries: { acr: { name: 'acr', url: 'registry.example.com', isDefault: true } },
      hosts: { lan: { name: 'lan', host: '192.168.1.10' } }
    });
    assert.equal(file, path.join(dir, 'mcp.json'));
    assert.equal(file, configPath());
    assert.equal(fileModeIsPrivate(file), true);
    if (process.platform !== 'win32') {
      assert.equal((await fs.stat(file)).mode & 0o777, 0o600);
      assert.equal((await fs.stat(dir)).mode & 0o777, 0o700);
    }

    const loaded = await loadConfig();
    assert.equal(loaded.config.version, 1);
    assert.equal(loaded.config.registries.acr.url, 'registry.example.com');
    assert.equal(loaded.config.hosts.lan.host, '192.168.1.10');
    assert.equal(loaded.config.update.endpoint, '');

    const mutated = await mutateConfig((config) => {
      config.update.endpoint = 'https://updates.example.com/latest.json';
    });
    assert.equal(mutated.config.update.endpoint, 'https://updates.example.com/latest.json');
    assert.equal((await loadConfig()).config.update.endpoint, 'https://updates.example.com/latest.json');
  } finally {
    restoreEnv();
  }
});

test('loadConfig 拒绝无效 JSON', async () => {
  await useTempConfigDir();
  try {
    await fs.mkdir(path.dirname(configPath()), { recursive: true });
    await fs.writeFile(configPath(), '{not json', 'utf8');
    await assert.rejects(loadConfig(), (error) => error.code === 'CONFIG_INVALID');
  } finally {
    restoreEnv();
  }
});
