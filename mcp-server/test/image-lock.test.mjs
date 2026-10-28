import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import {
  buildImageProbeScript,
  evaluateImagePrecheck,
  loadImageLock,
  parseImageProbe,
  validateImageLock
} from '../src/image-lock.mjs';
import { DIGEST_A, DIGEST_B, makeTempDir } from './helpers.mjs';

const sha = (seed) => `sha256:${seed.repeat(64)}`;
const digestOf = (value) => `sha256:${value}`;

function makeLock(overrides = {}) {
  return {
    schema: 'deploy.desktop/image-lock/v1',
    product: 'demo-product',
    requiredPlatforms: ['linux/amd64', 'linux/arm64'],
    images: overrides.images ?? [
      {
        name: 'db',
        ref: 'demo/db:1.0',
        digest: digestOf(DIGEST_A),
        platforms: {
          'linux/amd64': { manifest: sha('1'), imageId: sha('2') },
          'linux/arm64': { manifest: sha('3'), imageId: sha('4') }
        }
      }
    ]
  };
}

const images = [{ name: 'db', imageRef: `demo/db:1.0@${digestOf(DIGEST_A)}`, required: true }];

test('validateImageLock 归一化平台标识并保留每架构镜像 ID', () => {
  const lock = validateImageLock(makeLock());
  assert.deepEqual(lock.requiredPlatforms, ['linux/amd64', 'linux/arm64']);
  assert.equal(lock.images[0].platforms['linux/arm64'].imageId, sha('4'));
});

test('validateImageLock 拒绝缺少必需平台的镜像', () => {
  const raw = makeLock();
  delete raw.images[0].platforms['linux/arm64'];
  assert.throws(() => validateImageLock(raw), /未声明 linux\/arm64/);
});

test('validateImageLock 拒绝非法摘要与格式', () => {
  const badDigest = makeLock();
  badDigest.images[0].digest = 'sha256:1234';
  assert.throws(() => validateImageLock(badDigest), /digest 必须是 sha256 摘要/);
  assert.throws(() => validateImageLock({ ...makeLock(), schema: 'other' }), /镜像锁格式/);
});

test('buildImageProbeScript 转义镜像引用并保留缺失回退', () => {
  const script = buildImageProbeScript([{ name: 'db', imageRef: `demo/db:1.0@${digestOf(DIGEST_A)}` }]);
  assert.match(script, /docker version --format '\{\{\.Server\.Os\}\}\/\{\{\.Server\.Arch\}\}'/);
  assert.match(script, new RegExp(`docker image inspect 'demo/db:1\\.0' `));
  assert.match(script, new RegExp(`docker image inspect 'demo/db:1\\.0@sha256:${DIGEST_A}' `));
  assert.match(script, /'0' 'tag'/);
  assert.match(script, /'0' 'ref'/);
  assert.match(script, /MISSING\|\|/);
  assert.match(script, /'\{\{\.Os\}\}\/\{\{\.Architecture\}\}\|\{\{\.Id\}\}\|\{\{json \.RepoDigests\}\}'/);
});

test('parseImageProbe 兼容 Docker 27 输出的 JSON 形式 RepoDigests', () => {
  const stdout = [
    'HOST|linux/x86_64',
    `IMG|0|ref|linux/amd64|${sha('3')}|["demo/db@${digestOf(DIGEST_A)}","mirror/db@${digestOf(DIGEST_A)}"]`,
    `IMG|1|ref|linux/amd64|${sha('4')}|[]`,
    `IMG|2|ref|linux/amd64|${sha('5')}|demo/db@${digestOf(DIGEST_B)}`
  ].join('\n');
  const parsed = parseImageProbe(stdout);
  assert.equal(parsed.hostPlatform, 'linux/amd64');
  assert.deepEqual(parsed.probes[0].repoDigests, [
    `demo/db@${digestOf(DIGEST_A)}`,
    `mirror/db@${digestOf(DIGEST_A)}`
  ]);
  assert.deepEqual(parsed.probes[1].repoDigests, []);
  assert.deepEqual(parsed.probes[2].repoDigests, [`demo/db@${digestOf(DIGEST_B)}`]);
});

test('parseImageProbe 归一化架构并识别缺失镜像', () => {
  const stdout = [
    'HOST|linux/x86_64',
    `IMG|0|tag|linux/amd64|${sha('2')}|demo/db@${digestOf(DIGEST_A)}`,
    'IMG|1|tag|MISSING|||'
  ].join('\n');
  const parsed = parseImageProbe(stdout);
  assert.equal(parsed.hostPlatform, 'linux/amd64');
  assert.equal(parsed.probes[0].source, 'tag');
  assert.equal(parsed.probes[0].imageId, sha('2'));
  assert.deepEqual(parsed.probes[0].repoDigests, [`demo/db@${digestOf(DIGEST_A)}`]);
  assert.equal(parsed.probes[1].imageId, '');
});

test('evaluateImagePrecheck 在 tag 与 digest 两种解析结果中按目标架构择优', () => {
  const lock = validateImageLock(makeLock());
  const probes = [
    { index: 0, source: 'tag', osArch: 'linux/amd64', imageId: sha('1'), repoDigests: [] },
    { index: 0, source: 'ref', osArch: 'linux/arm64', imageId: sha('4'), repoDigests: [] }
  ];
  const result = evaluateImagePrecheck({ lock, images, hostPlatform: 'linux/arm64', probes });
  assert.deepEqual(result.problems, []);
  assert.equal(result.checks[0].osArch, 'linux/arm64');
  assert.equal(result.checks[0].imageId, sha('4'));
});

test('evaluateImagePrecheck 双架构匹配时通过', () => {
  const lock = validateImageLock(makeLock());
  const result = evaluateImagePrecheck({
    lock,
    images,
    hostPlatform: 'linux/amd64',
    probes: [{ index: 0, osArch: 'linux/amd64', imageId: sha('2'), repoDigests: [] }]
  });
  assert.deepEqual(result.problems, []);
  assert.equal(result.checks[0].expectedImageId, sha('2'));
});

test('evaluateImagePrecheck 报告缺失、架构不符与身份不一致', () => {
  const lock = validateImageLock(makeLock());
  const missing = evaluateImagePrecheck({ lock, images, hostPlatform: 'linux/amd64', probes: [] });
  assert.match(missing.problems.join(), /未导入镜像 db/);
  const wrongArch = evaluateImagePrecheck({
    lock,
    images,
    hostPlatform: 'linux/arm64',
    probes: [{ index: 0, osArch: 'linux/amd64', imageId: sha('2'), repoDigests: [] }]
  });
  assert.match(wrongArch.problems.join(), /架构为 linux\/amd64，目标机为 linux\/arm64/);
  const wrongId = evaluateImagePrecheck({
    lock,
    images,
    hostPlatform: 'linux/amd64',
    probes: [{ index: 0, osArch: 'linux/amd64', imageId: sha('9'), repoDigests: [] }]
  });
  assert.match(wrongId.problems.join(), /内容与镜像锁不一致/);
});

test('evaluateImagePrecheck 校验 repo digest 且非必需镜像只告警', () => {
  const lock = validateImageLock(makeLock());
  const digestMismatch = evaluateImagePrecheck({
    lock,
    images,
    hostPlatform: 'linux/amd64',
    probes: [{ index: 0, osArch: 'linux/amd64', imageId: sha('2'), repoDigests: [`demo/db@${digestOf(DIGEST_B)}`] }]
  });
  assert.match(digestMismatch.problems.join(), /repo digest 与镜像锁不一致/);
  const optional = evaluateImagePrecheck({
    lock,
    images: [{ ...images[0], required: false }],
    hostPlatform: 'linux/amd64',
    probes: []
  });
  assert.deepEqual(optional.problems, []);
  assert.match(optional.warnings.join(), /未导入镜像 db/);
});

test('loadImageLock 在缺少锁文件时给出可解释提示', async () => {
  const root = await makeTempDir();
  await fs.mkdir(path.join(root, 'deploy'), { recursive: true });
  const bundle = { root, artifact: () => null };
  const missing = await loadImageLock(bundle);
  assert.equal(missing.present, false);
  assert.match(missing.warning, /未附带镜像锁/);
  await fs.writeFile(path.join(root, 'deploy/images.lock.json'), JSON.stringify(makeLock()));
  const loaded = await loadImageLock(bundle);
  assert.equal(loaded.present, true);
  assert.equal(loaded.lock.images[0].name, 'db');
});
