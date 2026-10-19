import assert from 'node:assert/strict';
import test from 'node:test';
import {
  appVersion,
  containerName,
  databaseContainer,
  resolveContainer,
  serviceKeysForComponent,
  updateProfile,
  validateProfile
} from '../src/profile.mjs';
import { makeProfile } from './helpers.mjs';

test('validateProfile 接受完整配置并补默认值', () => {
  const profile = validateProfile(makeProfile());
  assert.equal(profile.containerPrefix, 'demo');
  assert.equal(profile.containers[1].protocol, 'TCP');
  assert.equal(profile.containers[1].description, '');
  assert.equal(profile.containers[2].port, null);
  assert.equal(profile.database.port, 2881);
});

test('validateProfile 拒绝非法容器前缀与产物名', () => {
  assert.throws(() => validateProfile(makeProfile({ containerPrefix: '-bad' })), /containerPrefix/);
  assert.throws(() => validateProfile(makeProfile({ backendArtifact: 'a/b.jar' })), /backendArtifact/);
});

test('validateProfile 校验数据库与容器关联', () => {
  assert.throws(
    () => validateProfile(makeProfile({ database: { serviceKey: 'missing', name: 'db', client: 'obclient', port: 2881 } })),
    /database\.serviceKey 未出现在 containers 中/
  );
  assert.throws(
    () => validateProfile(makeProfile({ database: { serviceKey: 'oceanbase', name: 'db-name', client: 'obclient', port: 2881 } })),
    /database\.name 只能包含/
  );
  assert.throws(
    () => validateProfile(makeProfile({ database: { serviceKey: 'oceanbase', name: 'db', client: 'obclient', port: 0 } })),
    /database\.port/
  );
});

test('validateProfile 拒绝重复服务键与非法端口', () => {
  const duplicate = makeProfile();
  duplicate.containers[1].serviceKey = 'oceanbase';
  assert.throws(() => validateProfile(duplicate), /serviceKey 重复/);

  const badPort = makeProfile();
  badPort.containers[1].port = 70000;
  assert.throws(() => validateProfile(badPort), /port 必须是 1-65535/);

  const badProtocol = makeProfile();
  badProtocol.containers[1].protocol = 'SCTP';
  assert.throws(() => validateProfile(badProtocol), /protocol 必须是 TCP 或 UDP/);
});

test('validateProfile 校验镜像包声明', () => {
  assert.throws(() => validateProfile(makeProfile({ images: [] })), /images 必须是非空数组/);
  assert.throws(() => validateProfile(makeProfile({ images: [{ file: 'a/b.tar', imageRef: 'x' }] })), /images\[0\]\.file/);
  assert.throws(
    () => validateProfile(makeProfile({ images: [{ file: 'a.tar', imageRef: 'x' }, { file: 'a.tar', imageRef: 'y' }] })),
    /file 重复/
  );
});

test('容器名与组分映射符合 profile 约定', () => {
  const profile = validateProfile(makeProfile());
  assert.equal(containerName(profile, 'frontend'), 'demo-frontend');
  assert.equal(databaseContainer(profile), 'demo-oceanbase');
  assert.equal(resolveContainer(profile, 'frontend'), 'demo-frontend');
  assert.equal(resolveContainer(profile, 'demo-frontend'), 'demo-frontend');
  assert.throws(() => resolveContainer(profile, 'missing'), /不存在服务/);

  assert.deepEqual(serviceKeysForComponent(profile, 'backend'), ['backend-java']);
  assert.deepEqual(serviceKeysForComponent(profile, 'gateway'), ['gateway-shard-1']);
  assert.deepEqual(serviceKeysForComponent(profile, 'frontend'), ['frontend']);
  assert.equal(serviceKeysForComponent(profile, 'config').length, profile.containers.length);
  assert.deepEqual(serviceKeysForComponent(profile, 'unknown'), []);
});

test('updateProfile 拒绝未知字段', async () => {
  await assert.rejects(updateProfile({ notAField: true }), /不支持的 profile 字段/);
});

test('appVersion 读取项目版本', async () => {
  assert.match(await appVersion(), /^\d+\.\d+\.\d+/);
});

test('profile.components 优先于旧的服务名前缀推断', () => {
  const profile = validateProfile(
    makeProfile({
      components: { backend: ['api'], gateway: ['gw-a', 'gw-b'], frontend: ['web'] }
    })
  );
  assert.deepEqual(serviceKeysForComponent(profile, 'backend'), ['api']);
  assert.deepEqual(serviceKeysForComponent(profile, 'gateway'), ['gw-a', 'gw-b']);
  assert.deepEqual(serviceKeysForComponent(profile, 'frontend'), ['web']);
});

test('validateProfile 拒绝非法 components 声明', () => {
  assert.throws(() => validateProfile(makeProfile({ components: [] })), /components 必须是对象/);
  assert.throws(
    () => validateProfile(makeProfile({ components: { backend: 'api' } })),
    /components\.backend 必须是数组/
  );
  assert.throws(
    () => validateProfile(makeProfile({ components: { backend: ['bad key'] } })),
    /不是合法的服务键/
  );
});
