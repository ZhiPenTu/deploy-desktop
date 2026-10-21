import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { Bundle, normalizeComponents, safeRelative } from '../src/bundle.mjs';
import { makeDescriptor, makeTempDir, writeBundle } from './helpers.mjs';

test('load 接受通用命名的合法交付包', async () => {
  const root = await makeTempDir();
  await writeBundle(root);
  const bundle = await Bundle.load(root);
  assert.equal(bundle.product(), 'demo-product');
  assert.equal(bundle.version(), '1.0.0');
  assert.equal(bundle.deploySourceRel(), 'deploy');
  assert.equal(bundle.containerName('api'), 'demo-api');
  assert.deepEqual(bundle.serviceKeysForComponent('site'), ['web']);
  assert.equal(bundle.hasBundleImages(), true);
  await bundle.verifyPayload();
});

test('load 不接受未知字段与错误 apiVersion', async () => {
  const root = await makeTempDir();
  await writeBundle(root, { mutate: async ({ root: dir, descriptor }) => {
    descriptor.extra = true;
  } });
  await assert.rejects(Bundle.load(root), (error) => error.code === 'BUNDLE_UNKNOWN_FIELD');

  const legacy = await makeTempDir();
  await writeBundle(legacy, { descriptor: { apiVersion: 'deploy.desktop/bundle/v1' } });
  await assert.rejects(Bundle.load(legacy), (error) => error.code === 'BUNDLE_VERSION_UNSUPPORTED');
});

test('load 拒绝绝对路径、上级目录与反斜杠', async () => {
  for (const bad of ['/etc/passwd', '../escape', 'app\\bin']) {
    const root = await makeTempDir();
    await writeBundle(root, { mutate: async ({ descriptor }) => {
      descriptor.artifacts[0].path = bad;
    } });
    await assert.rejects(Bundle.load(root), /产物部署路径|不合法/);
  }
  assert.throws(() => safeRelative('/abs', '路径'), /绝对路径/);
});

test('load 拒绝重复产物标识、重复部署位置与未覆盖文件', async () => {
  const duplicateKey = await makeTempDir();
  await writeBundle(duplicateKey, { mutate: async ({ descriptor }) => {
    descriptor.artifacts[1].key = descriptor.artifacts[0].key;
  } });
  await assert.rejects(Bundle.load(duplicateKey), /重复声明产物/);

  const duplicateTarget = await makeTempDir();
  await writeBundle(duplicateTarget, { mutate: async ({ descriptor }) => {
    descriptor.artifacts[1].path = descriptor.artifacts[0].path;
  } });
  await assert.rejects(Bundle.load(duplicateTarget), /重复声明部署位置/);

  const uncovered = await makeTempDir();
  await writeBundle(uncovered, { contents: { 'app/extra.txt': 'x' } });
  await assert.rejects(Bundle.load(uncovered), /未归属于任何产物或镜像/)
});

test('load 拒绝组件引用未声明的产物或服务', async () => {
  const root = await makeTempDir();
  await writeBundle(root, { mutate: async ({ descriptor }) => {
    descriptor.components[0].services.push('ghost');
  } });
  await assert.rejects(Bundle.load(root), /未声明的服务 ghost/);
});

test('verifyPayload 校验大小与摘要', async () => {
  const root = await makeTempDir();
  await writeBundle(root);
  const bundle = await Bundle.load(root);
  await fs.writeFile(path.join(root, 'app/config/app.yml'), 'tampered\n');
  await assert.rejects(bundle.verifyPayload(), (error) => error.code === 'BUNDLE_SIZE_MISMATCH' || error.code === 'BUNDLE_DIGEST_MISMATCH');

  const missing = await makeTempDir();
  await writeBundle(missing);
  await fs.rm(path.join(missing, 'deploy/db/init.sql'));
  const missingBundle = await Bundle.load(missing);
  await assert.rejects(missingBundle.verifyPayload(), (error) => error.code === 'BUNDLE_FILE_MISSING');
});

test('verifyPayload 拒绝未声明的多余文件', async () => {
  const root = await makeTempDir();
  await writeBundle(root);
  await fs.writeFile(path.join(root, 'app/extra.bin'), 'x');
  const bundle = await Bundle.load(root);
  await assert.rejects(bundle.verifyPayload(), (error) => error.code === 'BUNDLE_EXTRA_FILE');
});

test('minToolVersion 高于当前版本时拒绝使用', async () => {
  const root = await makeTempDir();
  await writeBundle(root, { descriptor: { metadata: { minToolVersion: '9.0.0' } } });
  await assert.rejects(Bundle.load(root, { appVersion: '0.3.0' }), (error) => error.code === 'TOOL_VERSION_TOO_LOW');
});

test('normalizeComponents 只接受交付包声明的组件', async () => {
  const root = await makeTempDir();
  await writeBundle(root);
  const bundle = await Bundle.load(root);
  assert.deepEqual(normalizeComponents(bundle, ['server', 'site', 'server']), ['server', 'site']);
  assert.deepEqual(normalizeComponents(bundle, ['image']), ['image']);
  assert.throws(() => normalizeComponents(bundle, ['backend']), /未声明组件 backend/);
  assert.throws(() => normalizeComponents(bundle, []), /至少选择一个/);
});

test('解析产物目标路径落在声明根内', async () => {
  const root = await makeTempDir();
  await writeBundle(root);
  const bundle = await Bundle.load(root);
  const artifacts = bundle.resolveArtifacts(bundle.selectComponents(['server', 'site']), '/opt/demo', '/opt/demo/deploy');
  const byKey = Object.fromEntries(artifacts.map((item) => [item.key, item.target]));
  assert.equal(byKey['server-bin'], '/opt/demo/bin');
  assert.equal(byKey['server-config'], '/opt/demo/config/app.yml');
  assert.equal(byKey['web-dist'], '/opt/demo/deploy/site');
});

test('非 compose 运行时明确拒绝', async () => {
  const root = await makeTempDir();
  await writeBundle(root, { descriptor: { runtime: { type: 'systemd' } } });
  await assert.rejects(Bundle.load(root), (error) => error.code === 'RUNTIME_UNSUPPORTED');
});

test('makeDescriptor 返回的基线描述符不含 files', () => {
  assert.deepEqual(makeDescriptor().files, []);
});
