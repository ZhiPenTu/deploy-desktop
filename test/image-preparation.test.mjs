import assert from 'node:assert/strict';
import { afterEach, beforeEach, test } from 'node:test';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { prepareImagePackages, importImagePackages } from '../src/features/deployment/services/imagePreparation.js';

const reference = `demo/api@sha256:${'a'.repeat(64)}`;
const context = {
  source: 'docker', bundleDir: '/bundle', imageDir: '/offline', installDir: '/srv/demo',
  references: [reference], remoteDir: '/srv/demo/bundle-images-task',
  target: { host: 'host.example.com', port: 2222, user: 'deploy', keyPath: '/key' }
};
const exported = { reference, fileName: 'api.tar', path: '/offline/api.tar', success: true };

beforeEach(() => {
  globalThis.window = {};
  globalThis.isTauri = true;
});
afterEach(() => {
  clearMocks();
  delete globalThis.window;
  delete globalThis.isTauri;
});

test('Docker transfer exports exact missing references and uses only confirmed packages and target', async () => {
  const calls = [];
  mockIPC((command, args) => {
    calls.push({ command, args });
    if (command === 'export_deployment_images') return [exported];
    return ['完成'];
  });
  const packages = await prepareImagePackages(context, () => {});
  assert.deepEqual(packages, { root: '/offline', files: ['api.tar'] });
  await importImagePackages(context, packages, () => {});
  assert.deepEqual(calls.map((item) => item.command), [
    'export_deployment_images', 'upload_remote_image_bundle', 'import_remote_image_bundle'
  ]);
  assert.deepEqual(calls[0].args, { bundleDir: '/bundle', references: [reference], ...context.target });
  assert.deepEqual(calls[1].args, { ...context.target, localDir: '/offline',
    remoteBundleDir: context.remoteDir, files: ['api.tar'], overwrite: false });
  assert.deepEqual(calls[2].args, { ...context.target, remoteBundleDir: context.remoteDir, files: ['api.tar'] });
});

test('failed, incomplete or wrong-reference exports cannot become upload packages', async () => {
  for (const results of [null, [], [{ ...exported, success: false, message: '本机缺少镜像' }],
    [{ ...exported, reference: 'demo/api:latest' }], [{ ...exported, path: '/offline/other.tar' }]]) {
    mockIPC(() => results);
    await assert.rejects(prepareImagePackages(context, () => {}));
  }
});

test('Windows export paths keep the exact archive name', async () => {
  mockIPC(() => [{ ...exported, path: 'C:\\offline\\api.tar' }]);
  assert.deepEqual(await prepareImagePackages(context, () => {}), { root: 'C:/offline', files: ['api.tar'] });
});

test('offline source sends only packages matched to missing images, once per package', async () => {
  mockIPC((command) => {
    if (command === 'scan_local_image_bundle') return { files: [{ name: 'api.tar.gz' }, { name: 'unrelated.tar' }] };
    if (command === 'inspect_deployment_review') return { images: [
      { imageRef: reference, localFile: 'images/api.tar.gz' },
      { imageRef: 'demo/worker:1', localFile: 'api.tar.gz' }
    ] };
    throw new Error('unexpected write');
  });
  assert.deepEqual(await prepareImagePackages({ ...context, source: 'offline',
    references: [reference, 'demo/worker:1'] }, () => {}), { root: '/offline', files: ['api.tar.gz'] });
});

test('unmatched offline packages are rejected before upload', async () => {
  mockIPC((command) => command === 'scan_local_image_bundle'
    ? { files: [{ name: 'unrelated.tar' }] }
    : { images: [{ imageRef: reference, localFile: '' }] });
  await assert.rejects(prepareImagePackages({ ...context, source: 'offline' }, () => {}), /未找到镜像/);
});

test('upload failure or incomplete acknowledgement prevents import', async () => {
  for (const result of [null, [], '完成', new Error('连接中断')]) {
    const calls = [];
    mockIPC((command) => {
      calls.push(command);
      if (result instanceof Error) throw result;
      return result;
    });
    await assert.rejects(importImagePackages(context, { root: '/offline', files: ['api.tar'] }, () => {}));
    assert.equal(calls.includes('import_remote_image_bundle'), false);
  }
});

test('import failure or missing result is not reported as complete', async () => {
  for (const result of [null, [], new Error('Docker 导入失败')]) {
    mockIPC((command) => {
      if (command === 'upload_remote_image_bundle') return ['完成'];
      if (result instanceof Error) throw result;
      return result;
    });
    await assert.rejects(importImagePackages(context, { root: '/offline', files: ['api.tar'] }, () => {}));
  }
});
