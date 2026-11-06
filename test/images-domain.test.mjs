import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  groupPullReferences,
  imagePreparationCounts,
  imageStatusDetail,
  imageStatusLabel,
  isDigestReference,
  pendingImages,
  pullableImages,
  registryOfReference
} from '../src/features/deployment/domain/images.js';

const images = [
  {
    imageRef: 'bitnamilegacy/kafka:4.0',
    status: 'missing',
    pullable: true,
    targetPlatform: 'linux/arm64'
  },
  {
    imageRef: 'bellsoft/liberica-openjdk-rocky:21-cds',
    status: 'platform_mismatch',
    platform: 'linux/amd64',
    targetPlatform: 'linux/arm64',
    pullable: true
  },
  {
    imageRef: `oceanbase/oceanbase-ce:4.4.2-lts@sha256:${'a'.repeat(64)}`,
    status: 'platform_mismatch',
    platform: 'linux/amd64',
    targetPlatform: 'linux/arm64',
    pullable: false
  },
  { imageRef: 'redis:7.2.8', status: 'ready', pullable: false },
  { imageRef: 'registry.example.com/demo/api:1.0', status: 'check_failed', pullable: false }
];

test('image statuses stay actionable and platform mismatch is explained', () => {
  assert.equal(imageStatusLabel('platform_mismatch'), '架构不匹配');
  assert.match(imageStatusDetail(images[1]), /linux\/amd64/);
  assert.match(imageStatusDetail(images[1]), /linux\/arm64/);
  assert.match(imageStatusDetail(images[2]), /该摘要不含目标架构/);
  assert.equal(pendingImages(images).length, 4);
  assert.deepEqual(
    pullableImages(images).map((image) => image.imageRef),
    ['bitnamilegacy/kafka:4.0', 'bellsoft/liberica-openjdk-rocky:21-cds']
  );
  assert.deepEqual(imagePreparationCounts(images), {
    missing: 1,
    mismatch: 2,
    failed: 1,
    invalid: 0,
    total: 4
  });
});

test('mixed registries group by registry and keep separate sources', () => {
  const sources = [
    { id: 'docker', name: 'Docker Hub', address: 'docker.io', username: 'reader', hasPassword: true },
    {
      id: 'private',
      name: '内部仓库',
      address: 'registry.example.com',
      username: 'reader',
      hasPassword: false
    }
  ];
  const mixed = [
    { imageRef: 'redis:7.2.8', status: 'missing', pullable: true },
    {
      imageRef: 'registry.example.com/demo/api:1.0',
      status: 'missing',
      pullable: true
    }
  ];
  const groups = groupPullReferences(mixed, sources, {
    'docker.io': { sourceId: 'docker' },
    'registry.example.com': { sourceId: 'private', credential: 'secret' }
  });
  assert.deepEqual(
    groups.map((group) => ({
      registry: group.registry,
      count: group.count,
      sourceId: group.sourceId,
      hasUsername: group.hasUsername,
      needsPassword: group.needsPassword,
      credential: group.credential
    })),
    [
      {
        registry: 'docker.io',
        count: 1,
        sourceId: 'docker',
        hasUsername: true,
        needsPassword: false,
        credential: ''
      },
      {
        registry: 'registry.example.com',
        count: 1,
        sourceId: 'private',
        hasUsername: true,
        needsPassword: true,
        credential: 'secret'
      }
    ]
  );
});

test('已保存密码的镜像源仍允许本次覆盖凭据', () => {
  const groups = groupPullReferences(
    [{ imageRef: 'registry.example.com/demo/api:1.0', status: 'missing', pullable: true }],
    [{ id: 'private', name: '内部仓库', address: 'registry.example.com', username: 'reader', hasPassword: true }],
    { 'registry.example.com': { sourceId: 'private', credential: '本次密码' } }
  );
  assert.equal(groups[0].hasUsername, true);
  assert.equal(groups[0].needsPassword, false);
  assert.equal(groups[0].credential, '本次密码');
});

test('public Docker Hub references normalize without a registry prefix', () => {
  assert.equal(registryOfReference('nginx:1.27-alpine'), 'docker.io');
  assert.equal(registryOfReference('bitnamilegacy/kafka:4.0'), 'docker.io');
  assert.equal(registryOfReference('localhost:5000/demo/api:1.0'), 'localhost:5000');
  assert.equal(registryOfReference('registry.example.com/demo/api:1.0'), 'registry.example.com');
});

test('摘要引用可识别，便于区分可传输与需目标机拉取的镜像', () => {
  assert.equal(isDigestReference('demo/api@sha256:' + 'a'.repeat(64)), true);
  assert.equal(isDigestReference('demo/api:1.4.0'), false);
  assert.equal(isDigestReference('registry.example.com/demo/api@sha256:' + 'a'.repeat(64)), true);
});
