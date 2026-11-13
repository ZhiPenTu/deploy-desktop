import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import {
  assertFullComponents,
  assertNoBlockingTransaction,
  buildImageOverrideDocument,
  parseTransactionStatus,
  publishedFrontendPort,
  redactDiagnostics,
  resolveStoredEffectiveCompose,
  uniqueImageRefs,
  validateMountContract
} from '../src/version-deploy.mjs';
import { Bundle } from '../src/bundle.mjs';

const mountCases = JSON.parse(readFileSync(new URL('../../fixtures/deployment/mounts.json', import.meta.url)));
test('未知或损坏的事务状态阻止新部署', () => {
  assert.throws(() => assertNoBlockingTransaction({ state: 'invalid', id: 'corrupt', phase: 'applying' }));
  assert.doesNotThrow(() => assertNoBlockingTransaction({ state: 'committed' }));
});
test('MCP 返回的事务日志隐藏凭据与私钥', () => {
  const text = redactDiagnostics('PASSWORD=example-private-value Bearer example-access-token https://name:pass@example.invalid/?token=value');
  assert.ok(!text.includes('example-private-value'));
  assert.ok(!text.includes('example-access-token'));
  assert.ok(!text.includes('name:pass'));
});
test('多端口映射优先使用声明的网页端口，歧义时阻止猜测', () => {
  const config = { services: { frontend: { ports: [
    { published: '9090', target: 9090, protocol: 'tcp' },
    { published: '8080', target: 80, protocol: 'tcp' }
  ] } } };
  assert.equal(publishedFrontendPort(config, 'frontend', 8080), 8080);
  assert.equal(publishedFrontendPort(config, 'frontend', 18080), null);
});
for (const item of mountCases.cases) {
  test(`持久状态保护契约：${item.name}（MCP）`, () => {
    if (item.valid) assert.doesNotThrow(() => validateMountContract(item.config, mountCases.contract));
    else assert.throws(() => validateMountContract(item.config, mountCases.contract));
  });
}

function bundleWithComponents(keys) {
  return {
    descriptor: {
      components: keys.map((key) => ({ key }))
    }
  };
}

test('完整版本部署只接受全部组件', () => {
  const bundle = bundleWithComponents(['server', 'site', 'config']);
  assert.deepEqual(assertFullComponents(bundle, ['config', 'server', 'site']), ['server', 'site', 'config']);
  assert.throws(
    () => assertFullComponents(bundle, ['server']),
    (error) => error.code === 'FULL_DEPLOY_REQUIRED' && /不支持部分组件更新/.test(error.message)
  );
  assert.throws(
    () => assertFullComponents(bundle, ['server', 'site', 'config', 'image']),
    (error) => error.code === 'FULL_DEPLOY_REQUIRED'
  );
});

test('事务状态覆盖阶段、失败原因和恢复错误', () => {
  const transaction = { phase: 'preparing', state: 'pending', updatedAt: '' };
  const status = [
    'phase=recovery_failed',
    'state=recovery_failed',
    'updatedAt=2026-11-13T01:00:00Z',
    `errorBase64=${Buffer.from('启动失败').toString('base64')}`,
    `recoveryErrorBase64=${Buffer.from('数据卷恢复失败').toString('base64')}`
  ].join('\n');
  parseTransactionStatus(status, transaction);
  assert.equal(transaction.phase, 'recovery_failed');
  assert.equal(transaction.state, 'recovery_failed');
  assert.equal(transaction.updatedAt, '2026-11-13T01:00:00Z');
  assert.equal(transaction.error, '启动失败');
  assert.equal(transaction.recoveryError, '数据卷恢复失败');
});

test('完整版本部署按预检镜像 ID 固定 Compose 服务镜像', () => {
  const document = buildImageOverrideDocument({
    services: {
      api: { image: 'demo/api@sha256:aaaaaaaa' },
      web: { image: 'demo/web@sha256:bbbbbbbb' },
      tools: { image: 'demo/tools@sha256:cccccccc', profiles: ['debug'] }
    }
  }, {
    checks: [
      { ref: 'demo/api@sha256:aaaaaaaa', present: true, imageId: 'sha256:1111' },
      { ref: 'demo/web@sha256:bbbbbbbb', present: true, imageId: 'sha256:2222' }
    ]
  });
  assert.deepEqual(JSON.parse(document), {
    services: {
      api: { image: 'sha256:1111', pull_policy: 'never' },
      web: { image: 'sha256:2222', pull_policy: 'never' }
    }
  });
  assert.equal(buildImageOverrideDocument({ services: {} }, { checks: [] }), '');
  assert.throws(
    () => buildImageOverrideDocument({ services: { app: { image: 'undeclared/app:2' } } }, { checks: [] }),
    (error) => error.code === 'SERVICE_IMAGE_NOT_READY'
  );
});

test('完整版本部署使用交付包声明的 Compose 变体并拒绝 install 根清单', () => {
  const bundle = new Bundle('/bundle', {
    runtime: {
      compose: 'deploy/docker-compose.yml',
      variant: 'deploy/docker-compose.lan.yml',
      services: [
        { serviceKey: 'api', role: 'app' },
        { serviceKey: 'db', role: 'data' }
      ],
      data: { volumes: [{ name: 'db-data', service: 'db', image: 'base' }] }
    },
    artifacts: [
      { source: 'deploy/docker-compose.yml', root: 'deploy' },
      { source: 'deploy/docker-compose.lan.yml', root: 'deploy' }
    ],
    images: []
  });
  assert.equal(bundle.effectiveCompose(), 'deploy/docker-compose.lan.yml');
  bundle.ensureFullDeployMetadata();

  bundle.descriptor.artifacts[1].root = 'install';
  assert.throws(
    () => bundle.ensureFullDeployMetadata(),
    /Compose 目录/
  );
});

test('旧部署记录的生效 Compose 缺失或越界时阻止部署', () => {
  assert.equal(
    resolveStoredEffectiveCompose(
      '/srv/app',
      '/srv/app/deploy',
      'deploy/docker-compose.lan.yml',
      'FOUND'
    ),
    'docker-compose.lan.yml'
  );
  assert.throws(
    () => resolveStoredEffectiveCompose(
      '/srv/app',
      '/srv/app/deploy',
      'deploy/docker-compose.lan.yml',
      'MISSING'
    ),
    (error) => error.code === 'OLD_COMPOSE_MISSING' && /生效 Compose 清单不存在/.test(error.message)
  );
  assert.throws(
    () => resolveStoredEffectiveCompose(
      '/srv/app',
      '/srv/app/deploy',
      '/srv/other/docker-compose.yml',
      'FOUND'
    ),
    (error) => error.code === 'OLD_COMPOSE_UNSUPPORTED'
  );
  assert.throws(
    () => resolveStoredEffectiveCompose(
      '/srv/app',
      '/srv/app/deploy',
      'deploy/../outside/docker-compose.yml',
      'FOUND'
    ),
    (error) => error.code === 'OLD_COMPOSE_UNSUPPORTED' && /无效跳转/.test(error.message)
  );
  assert.throws(
    () => resolveStoredEffectiveCompose(
      '/srv/app',
      '/srv/app/deploy',
      '/srv/app/deploy/docker-compose.yml',
      'OUTSIDE'
    ),
    (error) => error.code === 'OLD_COMPOSE_UNSUPPORTED'
  );
});

test('旧部署镜像预检只包含实际启用的服务', () => {
  assert.deepEqual(
    uniqueImageRefs({
      services: {
        api: { image: 'demo/api:1' },
        tools: { image: 'demo/tools:debug', profiles: ['debug'] }
      }
    }),
    ['demo/api:1']
  );
});
