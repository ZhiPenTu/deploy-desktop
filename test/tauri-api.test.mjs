import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
import { afterEach, beforeEach, test } from 'node:test';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import * as api from '../src/api/tauri.js';

beforeEach(() => {
  globalThis.window = { crypto: webcrypto };
  globalThis.isTauri = false;
});

afterEach(() => {
  clearMocks();
  delete globalThis.window;
  delete globalThis.isTauri;
});

test('browser preview starts with empty inventories and no active project', async () => {
  assert.equal(api.isDesktopApp(), false);
  assert.deepEqual(await api.listProjects(), { schemaVersion: 1, activeProjectId: '', projects: [] });
  assert.deepEqual(await api.getActiveProjectContext(), { project: null, bundle: null, deployDir: '' });
  assert.deepEqual(await api.listArtifactSources(), []);
  assert.deepEqual(await api.listArtifactTokens(), []);
  assert.deepEqual(await api.listRegistrySources(), []);
  assert.deepEqual(await api.listConnectionInventory(), { credentials: [], hosts: [], activeHostId: '' });
});

test('every desktop operation rejects in the browser instead of reporting simulated success', async () => {
  const previewHelpers = new Set([
    'isDesktopApp', 'listProjects', 'getActiveProjectContext', 'listArtifactSources', 'listArtifactTokens',
    'listRegistrySources',
    'listConnectionInventory', 'openArtifactRunUrl', 'reportClientEvent', 'onImageLibraryProgress',
    'onImageManagerProgress', 'onRegistryProgress', 'onRemoteUpdateProgress', 'setCommandTrace'
  ]);
  for (const [name, operation] of Object.entries(api)) {
    if (previewHelpers.has(name)) continue;
    await assert.rejects(operation(), /暂不可用，请在桌面应用中重试/, name);
  }
});

test('desktop updates preserve target, component, backup and dry-run arguments and backend outcomes', async () => {
  globalThis.isTauri = true;
  const calls = [];
  const outcome = { success: false, output: '', error: '目标环境尚未就绪' };
  mockIPC((command, args) => { calls.push({ command, args }); return outcome; });
  const components = ['backend', 'config'];
  assert.equal(await api.updateRemoteDeployment('host.example.com', 2222, 'deploy', '/key', '/bundle', '/srv/app', components, false, true), outcome);
  assert.equal(await api.updateLocalDeployment('/bundle', '/srv/app', components), outcome);
  assert.deepEqual(calls, [
    { command: 'update_remote_deployment', args: {
      host: 'host.example.com', port: 2222, user: 'deploy', keyPath: '/key',
      bundleDir: '/bundle', installDir: '/srv/app', components, backup: false, dryRun: true
    } },
    { command: 'update_local_deployment', args: {
      bundleDir: '/bundle', installDir: '/srv/app', components, backup: true, dryRun: false
    } }
  ]);
  mockIPC(() => { throw new Error('连接失败'); });
  await assert.rejects(api.checkLocalEnv(), /连接失败/);
});

test('project context, update checks and progress channels use the desktop bridge', async () => {
  globalThis.isTauri = true;
  const events = [];
  const calls = [];
  const store = { schemaVersion: 1, activeProjectId: 'p-1', projects: [{ id: 'p-1', name: '生产环境' }] };
  const context = { project: store.projects[0], bundle: null, deployDir: '/srv/app/deploy' };
  const release = { version: '1.0.0' };
  mockIPC((command, args) => {
    calls.push(command);
    if (command === 'list_projects') return store;
    if (command === 'get_active_project_context') return context;
    if (command === 'activate_project_bundle') {
      assert.deepEqual(args, { projectId: 'p-1', bundleDir: '/bundle/1.0.0', version: '1.0.0' });
      return store;
    }
    if (command === 'check_app_update') return release;
    if (command === 'download_artifact') {
      assert.equal(args.id, 'source');
      assert.equal(args.release, release);
    }
    window.__TAURI_INTERNALS__.runCallback(args.onEvent.id, { index: 0, message: { command, percent: 50 } });
    window.__TAURI_INTERNALS__.runCallback(args.onEvent.id, { index: 1, end: true });
    return null;
  });
  assert.deepEqual(await api.listProjects(), store);
  assert.deepEqual(await api.getActiveProjectContext(), context);
  assert.deepEqual(await api.activateProjectBundle('p-1', '/bundle/1.0.0', '1.0.0'), store);
  assert.equal(await api.checkAppUpdate(), release);
  await api.installAppUpdate((event) => events.push(event));
  await api.downloadArtifact('source', release, (event) => events.push(event));
  assert.deepEqual(calls, [
    'list_projects', 'get_active_project_context', 'activate_project_bundle',
    'check_app_update', 'install_app_update', 'download_artifact'
  ]);
  assert.deepEqual(events, [{ command: 'install_app_update', percent: 50 }, { command: 'download_artifact', percent: 50 }]);
});

test('progress listeners remain optional in preview and can unsubscribe on desktop', async () => {
  const events = [];
  const stopPreview = await api.onRemoteUpdateProgress((event) => events.push(event));
  stopPreview();
  assert.deepEqual(events, []);
  globalThis.isTauri = true;
  mockIPC(() => {}, { shouldMockEvents: true });
  const stop = await api.onRemoteUpdateProgress((event) => events.push(event));
  await emit('remote-update-progress', { step: 'backup' });
  await stop();
  assert.equal(window.__TAURI_INTERNALS__.callbacks.size, 0);
  assert.deepEqual(events, [{ step: 'backup' }]);
});

test('pending reads are shared across remounts while different targets remain independent', async () => {
  globalThis.isTauri = true;
  const pending = [];
  mockIPC((command, args) => new Promise((resolve) => pending.push({ command, args, resolve })));
  const first = api.getRemoteContainersStatus('host.example.com', 22, 'deploy', null, '/srv/app');
  const remounted = api.getRemoteContainersStatus('host.example.com', 22, 'deploy', null, '/srv/app');
  const other = api.getRemoteContainersStatus('other.example.com', 22, 'deploy', null, '/srv/app');
  assert.equal(pending.length, 2);
  pending[0].resolve([{ name: 'service' }]);
  pending[1].resolve([]);
  assert.deepEqual(await first, await remounted);
  assert.deepEqual(await other, []);
});

test('failed reads are not cached and explicit retry executes again', async () => {
  globalThis.isTauri = true;
  let reject;
  let calls = 0;
  mockIPC((command) => {
    if (command === 'log_client_event') return null;
    calls += 1;
    return new Promise((_, fail) => { reject = fail; });
  });
  const first = api.checkRemoteEnv('host.example.com', 22, 'deploy');
  const second = api.checkRemoteEnv('host.example.com', 22, 'deploy');
  const checked = Promise.allSettled([first, second]);
  reject(new Error('连接超时'));
  assert.ok((await checked).every((item) => item.status === 'rejected'));
  assert.equal(calls, 1);
  mockIPC((command) => {
    if (command === 'log_client_event') return null;
    calls += 1;
    return { docker_running: true };
  });
  assert.equal((await api.checkRemoteEnv('host.example.com', 22, 'deploy')).docker_running, true);
  assert.equal(calls, 2);
});

test('mutating operations are never coalesced', async () => {
  globalThis.isTauri = true;
  let calls = 0;
  mockIPC(() => { calls += 1; return { success: true }; });
  await Promise.all([
    api.remoteDockerComposeAction('host.example.com', 22, 'deploy', null, '/srv/app', 'restart'),
    api.remoteDockerComposeAction('host.example.com', 22, 'deploy', null, '/srv/app', 'restart')
  ]);
  assert.equal(calls, 2);
});
