import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { PROJECT_ROOT } from '../src/config.mjs';
import { DeployError } from '../src/errors.mjs';
import {
  isValidDigest,
  loadManifestSummary,
  normalizeRegistry,
  selectImages,
  validateManifest,
  writeManifest
} from '../src/manifest.mjs';
import { DIGEST_A, DIGEST_B, makeManifest, makeTempDir } from './helpers.mjs';

test('normalizeRegistry 接受带协议与末尾斜杠的主机名', () => {
  assert.equal(normalizeRegistry('https://Registry.Example.com/'), 'registry.example.com');
  assert.equal(normalizeRegistry('127.0.0.1:5000'), '127.0.0.1:5000');
  assert.throws(() => normalizeRegistry('https://registry.example.com/team'), /仓库地址无效/);
  assert.throws(() => normalizeRegistry('registry example.com'), /仓库地址无效/);
});

test('isValidDigest 只接受 64 位小写 sha256', () => {
  assert.equal(isValidDigest(`sha256:${DIGEST_A}`), true);
  assert.equal(isValidDigest(`sha256:${DIGEST_A.toUpperCase()}`), false);
  assert.equal(isValidDigest('sha256:abc'), false);
  assert.equal(isValidDigest(`md5:${DIGEST_A}`), false);
});

test('validateManifest 解析完整镜像引用', () => {
  const result = validateManifest(makeManifest());
  assert.equal(result.registry, 'registry.example.com');
  assert.equal(result.namespace, 'team');
  assert.equal(result.images.length, 2);
  assert.equal(result.images[0].reference, `registry.example.com/team/backend@sha256:${DIGEST_A}`);
  assert.equal(result.images[0].platform, 'linux/amd64');
  assert.equal(result.images[0].sizeBytes, 1024);
});

test('validateManifest 支持覆盖 registry 与 namespace', () => {
  const result = validateManifest(makeManifest(), {
    registryOverride: 'https://harbor.example.com/',
    namespaceOverride: '/other-ns/'
  });
  assert.equal(result.registry, 'harbor.example.com');
  assert.equal(result.images[0].reference, `harbor.example.com/other-ns/backend@sha256:${DIGEST_A}`);
});

test('validateManifest 拒绝未知字段', () => {
  assert.throws(() => validateManifest(makeManifest({ extra: true })), /未知字段: extra/);
  assert.throws(() => validateManifest(makeManifest({ metadata: { typo: 'x' } })), /metadata 包含未知字段: typo/);
  assert.throws(() => validateManifest(makeManifest({ spec: { unknown: 1 } })), /spec 包含未知字段: unknown/);
  const manifest = makeManifest();
  manifest.spec.images[0].unexpected = true;
  assert.throws(() => validateManifest(manifest), /images\[0\] 包含未知字段: unexpected/);
});

test('validateManifest 强制 sha256 digest', () => {
  const missing = makeManifest();
  delete missing.spec.images[0].digest;
  assert.throws(() => validateManifest(missing), /缺少 digest/);

  const invalid = makeManifest();
  invalid.spec.images[0].digest = 'sha256:1234';
  assert.throws(() => validateManifest(invalid), /digest 无效/);
});

test('validateManifest 拒绝重复名称与非法 repository', () => {
  const duplicate = makeManifest();
  duplicate.spec.images[1].name = duplicate.spec.images[0].name;
  assert.throws(() => validateManifest(duplicate), /镜像名称重复/);

  const traversal = makeManifest();
  traversal.spec.images[0].repository = '../evil';
  assert.throws(() => validateManifest(traversal), /repository 无效/);
});

test('validateManifest 校验 platform 与 artifacts', () => {
  const badPlatform = makeManifest({ spec: { platform: 'linux' } });
  assert.throws(() => validateManifest(badPlatform), /平台格式无效/);

  const badArtifact = makeManifest({ spec: { artifacts: [{ name: 'compose', extra: true }] } });
  assert.throws(() => validateManifest(badArtifact), /artifacts\[0\] 包含未知字段/);
});

test('selectImages 按名称选择并拒绝未知镜像', () => {
  const summary = validateManifest(makeManifest());
  assert.equal(selectImages(summary).length, 2);
  assert.equal(selectImages(summary, ['frontend'])[0].name, 'frontend');
  assert.throws(() => selectImages(summary, ['missing']), /清单中不存在镜像/);
});

test('loadManifestSummary 可加载仓库示例清单', async () => {
  const summary = await loadManifestSummary(path.join(PROJECT_ROOT, 'examples', 'manifest.example.yaml'));
  assert.equal(summary.images.length, 2);
  assert.equal(summary.registry, 'registry.cn-hangzhou.aliyuncs.com');
  assert.ok(summary.totalSizeBytes > 0);
  assert.ok(Array.isArray(summary.warnings));
});

test('loadManifestSummary 拒绝不存在的文件', async () => {
  await assert.rejects(
    loadManifestSummary('/definitely/not/here/manifest.yaml'),
    (error) => error instanceof DeployError && /读取本地清单失败/.test(error.message)
  );
});

test('writeManifest 写入 YAML 并保护已有文件', async () => {
  const dir = await makeTempDir();
  const target = path.join(dir, 'manifest.yaml');
  const first = await writeManifest(target, makeManifest());
  assert.equal(first.images.length, 2);
  const content = await fs.readFile(target, 'utf8');
  assert.match(content, /apiVersion: deploy\.desktop\/v1/);

  await assert.rejects(writeManifest(target, makeManifest()), /已存在/);
  const second = await writeManifest(target, makeManifest({ metadata: { version: '2.0.0' } }), { overwrite: true });
  assert.equal(second.images.length, 2);
  assert.match(await fs.readFile(target, 'utf8'), /version: 2\.0\.0/);
});
