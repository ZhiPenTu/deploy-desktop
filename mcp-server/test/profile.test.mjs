import assert from 'node:assert/strict';
import test from 'node:test';
import { Bundle } from '../src/bundle.mjs';
import {
  buildProfile,
  containerName,
  databaseContainer,
  resolveContainer,
  resolveServiceKey,
  serviceKeysForComponent,
  validateProfile
} from '../src/profile.mjs';
import { makeProject, makeTempDir, writeBundle } from './helpers.mjs';

async function demoProfile() {
  const root = await makeTempDir();
  await writeBundle(root);
  const bundle = await Bundle.load(root);
  return validateProfile(buildProfile(makeProject({ lastBundleDir: root }), bundle));
}

test('buildProfile 完全按交付包推导运行时视图', async () => {
  const profile = await demoProfile();
  assert.equal(profile.containerPrefix, 'demo');
  assert.equal(profile.deployDir, '/opt/demo/deploy');
  assert.equal(profile.deploySourceRel, 'deploy');
  assert.equal(profile.installDir, '/opt/demo');
  assert.equal(profile.compose.file, '/opt/demo/deploy/docker-compose.yml');
  assert.equal(profile.compose.fileRelative, 'docker-compose.yml');
  assert.equal(profile.containers[0].protocol, 'TCP');
  assert.equal(profile.containers[0].description, '');
  assert.equal(profile.database.port, 2881);
  assert.deepEqual(profile.components.map((item) => item.key), ['server', 'site', 'config']);
  assert.deepEqual(profile.images.map((item) => item.file), ['db.tar']);
  assert.equal(profile.steps.length, 2);
});

test('容器名与组件服务映射来自交付包声明', async () => {
  const profile = await demoProfile();
  assert.equal(containerName(profile, 'web'), 'demo-web');
  assert.equal(databaseContainer(profile), 'demo-db');
  assert.equal(resolveContainer(profile, 'web'), 'demo-web');
  assert.equal(resolveContainer(profile, 'demo-web'), 'demo-web');
  assert.equal(resolveServiceKey(profile, 'demo-web'), 'web');
  assert.throws(() => resolveContainer(profile, 'missing'), /不存在服务/);

  assert.deepEqual(serviceKeysForComponent(profile, 'server'), ['api']);
  assert.deepEqual(serviceKeysForComponent(profile, 'site'), ['web']);
  assert.equal(serviceKeysForComponent(profile, 'config').length, profile.containers.length);
  assert.deepEqual(serviceKeysForComponent(profile, 'unknown'), []);
});

test('自定义容器名优先于前缀拼接', async () => {
  const root = await makeTempDir();
  await writeBundle(root, {
    descriptor: {
      runtime: {
        containerPrefix: 'demo',
        services: [
          { key: 'api', label: '接口', port: 8000, protocol: 'TCP', container: 'custom-api' },
          { key: 'db', label: '数据库', port: 2881, protocol: 'TCP' }
        ],
        database: { serviceKey: 'db', name: 'demo_db', client: 'obclient', port: 2881 }
      },
      components: [
        { key: 'server', label: '服务端', artifacts: ['server-bin', 'server-config'], services: ['api'] },
        { key: 'config', label: '部署配置', artifacts: ['compose', 'db-config'], services: [] }
      ]
    }
  });
  const bundle = await Bundle.load(root);
  const profile = buildProfile(makeProject({ lastBundleDir: root }), bundle);
  assert.equal(containerName(profile, 'api'), 'custom-api');
  assert.equal(containerName(profile, 'db'), 'demo-db');
});

test('validateProfile 拒绝非法容器前缀与非绝对目录', async () => {
  const profile = await demoProfile();
  assert.throws(() => validateProfile({ ...profile, containerPrefix: '-bad' }), /容器前缀/);
  assert.throws(() => validateProfile({ ...profile, installDir: 'relative/dir' }), /绝对路径/);
});
