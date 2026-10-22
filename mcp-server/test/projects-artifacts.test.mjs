import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { registerTools } from '../src/tools.mjs';
import { makeTempDir, writeBundle } from './helpers.mjs';

const execFileAsync = promisify(execFile);
const ORIGINAL_APP_CONFIG_DIR = process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;

function captureTools() {
  const tools = new Map();
  registerTools({
    registerTool(name, definition, handler) {
      assert.equal(tools.has(name), false, `重复注册工具: ${name}`);
      tools.set(name, { definition, handler });
    }
  });
  return tools;
}

const tools = captureTools();

async function call(name, args = {}) {
  const result = await tools.get(name).handler(args, {});
  return { result, body: JSON.parse(result.content[0].text) };
}

async function withConfigDir(fn) {
  const dir = await makeTempDir('deploy-mcp-project-');
  process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = dir;
  try {
    return await fn(dir);
  } finally {
    if (ORIGINAL_APP_CONFIG_DIR === undefined) delete process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
    else process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = ORIGINAL_APP_CONFIG_DIR;
    await fs.rm(dir, { recursive: true, force: true });
  }
}

async function writeConnections(dir) {
  await fs.writeFile(path.join(dir, 'connections.json'), JSON.stringify({
    hosts: [
      { id: 'host-1', kind: 'ssh', name: 'lan-01', address: '192.168.1.20', port: 22, credentialId: 'cred-1', notes: '' }
    ],
    credentials: [{ id: 'cred-1', username: 'root', privateKeyPath: '/home/test/.ssh/id_ed25519' }]
  }));
}

async function archiveBundle(sourceDir, target) {
  await execFileAsync('tar', ['-czf', target, '-C', sourceDir, '.'], {
    env: { ...process.env, COPYFILE_DISABLE: '1' }
  });
  return fs.readFile(target);
}

test('项目可创建、编辑、切换、绑定交付包并删除', async () => {
  await withConfigDir(async (dir) => {
    await writeConnections(dir);
    await fs.writeFile(path.join(dir, 'artifact-sources.json'), JSON.stringify({
      sources: [{ id: 'src-1', name: '主制品源', type: 'yunxiaoFlow', orgId: 'org', pipelineId: 'pipe', artifactName: 'deploy-bundle' }]
    }));

    const created = await call('deploy_project_create', {
      name: '演示项目', host: 'lan-01', installDir: '/opt/demo/', source: '主制品源'
    });
    assert.equal(created.result.isError, undefined);
    assert.equal(created.body.project.hostId, 'host-1');
    assert.equal(created.body.project.sourceId, 'src-1');
    assert.equal(created.body.project.installDir, '/opt/demo');
    assert.equal(created.body.activeProjectId, created.body.project.id);

    const updated = await call('deploy_project_update', {
      project: created.body.project.id, name: '演示项目二', source: ''
    });
    assert.equal(updated.body.project.name, '演示项目二');
    assert.equal(updated.body.project.sourceId, '');

    const empty = await call('deploy_project_update', { project: created.body.project.id });
    assert.equal(empty.body.code, 'PROJECT_UPDATE_EMPTY');

    const bundleDir = await makeTempDir('deploy-mcp-bundle-');
    try {
      await writeBundle(bundleDir, { descriptor: { metadata: { product: 'demo-product', version: '1.2.3' } } });

      const mismatch = await call('deploy_project_bind_bundle', {
        project: created.body.project.id, bundleDir, version: '9.9.9'
      });
      assert.equal(mismatch.body.code, 'ARTIFACT_VERSION_MISMATCH');

      const bound = await call('deploy_project_bind_bundle', {
        project: '演示项目二', bundleDir, version: '1.2.3'
      });
      assert.equal(bound.body.project.product, 'demo-product');
      assert.equal(bound.body.project.lastVersion, '1.2.3');
      assert.equal(bound.body.project.lastBundleDir, path.resolve(bundleDir));
      assert.equal(bound.body.bundle.version, '1.2.3');

      const otherDir = await makeTempDir('deploy-mcp-other-');
      try {
        await writeBundle(otherDir, { descriptor: { metadata: { product: 'other-product', version: '1.2.4' } } });
        const productMismatch = await call('deploy_project_bind_bundle', {
          project: created.body.project.id, bundleDir: otherDir
        });
        assert.equal(productMismatch.body.code, 'PRODUCT_MISMATCH');
      } finally {
        await fs.rm(otherDir, { recursive: true, force: true });
      }
    } finally {
      await fs.rm(bundleDir, { recursive: true, force: true });
    }

    const second = await call('deploy_project_create', {
      name: '第二项目', host: '192.168.1.20', installDir: '/opt/second'
    });
    assert.equal(second.body.project.hostId, 'host-1');

    const activated = await call('deploy_project_activate', { project: '第二项目' });
    assert.equal(activated.body.activeProjectId, second.body.project.id);

    const denied = await call('deploy_project_remove', { project: '第二项目' });
    assert.equal(denied.body.code, 'CONFIRM_REQUIRED');

    const removed = await call('deploy_project_remove', { project: '第二项目', confirm: true });
    assert.equal(removed.body.removed.name, '第二项目');
    assert.equal(removed.body.activeProjectId, created.body.project.id);
    assert.equal((await call('deploy_project_list')).body.projects.length, 1);
  });
});

test('制品源支持新增、令牌保存、编辑、清除令牌与删除', async () => {
  await withConfigDir(async (dir) => {
    const created = await call('deploy_artifact_source_save', {
      name: '云效制品', type: 'yunxiaoFlow', orgId: 'org', pipelineId: 'pipe', token: 'secret-token'
    });
    assert.equal(created.result.isError, undefined);
    assert.equal(created.body.source.hasToken, true);
    assert.equal('token' in created.body.source, false);

    const sourcesText = await fs.readFile(path.join(dir, 'artifact-sources.json'), 'utf8');
    assert.equal(sourcesText.includes('secret-token'), false);
    const tokens = JSON.parse(await fs.readFile(path.join(dir, 'artifact-tokens.json'), 'utf8'));
    assert.equal(tokens[created.body.source.id], 'secret-token');
    if (process.platform !== 'win32') {
      const mode = (await fs.stat(path.join(dir, 'artifact-tokens.json'))).mode & 0o777;
      assert.equal(mode, 0o600);
    }

    const listed = await call('deploy_artifact_source_list');
    assert.equal(JSON.stringify(listed.body).includes('secret-token'), false);
    assert.equal(listed.body.sources.length, 1);

    const renamed = await call('deploy_artifact_source_save', {
      id: created.body.source.id, name: '云效制品二'
    });
    assert.equal(renamed.body.source.name, '云效制品二');
    assert.equal(renamed.body.source.orgId, 'org');
    assert.equal(renamed.body.source.hasToken, true);

    const cleared = await call('deploy_artifact_source_save', {
      id: created.body.source.id, name: '云效制品二', clearToken: true
    });
    assert.equal(cleared.body.source.hasToken, false);

    const insecure = await call('deploy_artifact_source_save', {
      name: '直连制品', type: 'url', version: '1.0.0', downloadUrl: 'http://example.test/bundle.tgz'
    });
    assert.equal(insecure.body.code, 'ARTIFACT_URL_INSECURE');

    const direct = await call('deploy_artifact_source_save', {
      name: '直连制品', type: 'url', version: '1.0.0', downloadUrl: 'https://example.test/bundle.tgz'
    });
    assert.equal(direct.body.source.type, 'url');

    const denied = await call('deploy_artifact_source_remove', { source: '直连制品' });
    assert.equal(denied.body.code, 'CONFIRM_REQUIRED');

    const removed = await call('deploy_artifact_source_remove', { source: '直连制品', confirm: true });
    assert.equal(removed.body.removed.name, '直连制品');
    assert.equal((await call('deploy_artifact_source_list')).body.sources.length, 1);
  });
});

test('制品源可复用访问令牌库的令牌，且不影响其他源', async () => {
  await withConfigDir(async (dir) => {
    await fs.writeFile(path.join(dir, 'artifact-sources.json'), JSON.stringify({
      sources: [{
        id: 'src-main',
        name: '主流水线',
        type: 'yunxiaoFlow',
        orgId: 'org',
        pipelineId: '111',
        artifactName: 'deploy-bundle',
        version: '',
        downloadUrl: '',
        tokenId: 'tok-shared',
        hasToken: true,
        lastVersion: '1.0.0',
        lastBundleDir: '/tmp/bundle'
      }]
    }));
    await fs.writeFile(path.join(dir, 'artifact-token-library.json'), JSON.stringify({
      tokens: [{ id: 'tok-shared', name: '云效令牌', createdAt: 1, updatedAt: 1 }]
    }));
    await fs.writeFile(path.join(dir, 'artifact-tokens.json'), JSON.stringify({ 'token:tok-shared': 'shared-token' }));

    const created = await call('deploy_artifact_source_save', {
      name: '局域网流水线',
      type: 'yunxiaoFlow',
      orgId: 'org',
      pipelineId: '5340973',
      artifactName: 'lan-deploy-bundle',
      tokenId: 'tok-shared'
    });
    assert.equal(created.result.isError, undefined);
    assert.equal(created.body.source.tokenId, 'tok-shared');
    assert.equal(created.body.source.hasToken, true);

    const file = JSON.parse(await fs.readFile(path.join(dir, 'artifact-sources.json'), 'utf8'));
    assert.equal(file.sources.length, 2);
    assert.equal(file.sources[0].tokenId, 'tok-shared');
    assert.equal(file.sources[0].lastVersion, '1.0.0');
    assert.equal(file.sources[1].tokenId, 'tok-shared');

    const tokens = JSON.parse(await fs.readFile(path.join(dir, 'artifact-tokens.json'), 'utf8'));
    assert.deepEqual(tokens, { 'token:tok-shared': 'shared-token' });

    const missing = await call('deploy_artifact_source_save', {
      name: '错误令牌源', type: 'yunxiaoFlow', orgId: 'org', pipelineId: '9', tokenId: 'tok-missing'
    });
    assert.equal(missing.body.code, 'ARTIFACT_TOKEN_NOT_FOUND');
    assert.equal((await call('deploy_artifact_source_list')).body.sources.length, 2);
  });
});

test('直接下载源可查看版本并下载校验交付包', async () => {
  await withConfigDir(async (dir) => {
    const sourceDir = await makeTempDir('deploy-mcp-artifact-bundle-');
    const archivePath = path.join(dir, 'fixture.tgz');
    const originalFetch = globalThis.fetch;
    try {
      await writeBundle(sourceDir, { descriptor: { metadata: { product: 'demo-product', version: '1.2.3' } } });
      const bytes = await archiveBundle(sourceDir, archivePath);
      let requested = '';
      globalThis.fetch = async (url) => {
        requested = String(url);
        return new Response(new Uint8Array(bytes), {
          status: 200,
          headers: { 'content-length': String(bytes.length), 'content-type': 'application/gzip' }
        });
      };

      const saved = await call('deploy_artifact_source_save', {
        name: '直连制品', type: 'url', version: '1.2.3', downloadUrl: 'https://example.test/bundle.tgz'
      });
      const releases = await call('deploy_artifact_releases', { source: '直连制品' });
      assert.equal(releases.body.items.length, 1);
      assert.equal(releases.body.items[0].version, '1.2.3');
      assert.equal(releases.body.items[0].localDir, '');

      const downloaded = await call('deploy_artifact_download', { source: '直连制品', version: '1.2.3' });
      assert.equal(downloaded.result.isError, undefined);
      assert.equal(requested, 'https://example.test/bundle.tgz');
      assert.equal(downloaded.body.version, '1.2.3');
      assert.equal(downloaded.body.md5, crypto.createHash('md5').update(bytes).digest('hex'));
      assert.equal(downloaded.body.files, 6);

      const manifest = JSON.parse(await fs.readFile(path.join(downloaded.body.dir, 'bundle.json'), 'utf8'));
      assert.equal(manifest.metadata.version, '1.2.3');
      assert.equal(path.basename(path.dirname(downloaded.body.dir)), saved.body.source.id);

      const sources = JSON.parse(await fs.readFile(path.join(dir, 'artifact-sources.json'), 'utf8'));
      assert.equal(sources.sources[0].lastVersion, '1.2.3');
      assert.equal(sources.sources[0].lastBundleDir, downloaded.body.dir);

      const cached = await call('deploy_artifact_releases', { source: '直连制品', cachedOnly: true });
      assert.equal(cached.body.items[0].localDir, downloaded.body.dir);

      const wrong = await call('deploy_artifact_download', { source: '直连制品', version: '9.9.9' });
      assert.equal(wrong.body.code, 'ARTIFACT_VERSION_MISMATCH');
    } finally {
      globalThis.fetch = originalFetch;
      await fs.rm(sourceDir, { recursive: true, force: true });
    }
  });
});

test('云效制品源可列出并下载构建物，拒绝非允许域名', async () => {
  await withConfigDir(async (dir) => {
    const sourceDir = await makeTempDir('deploy-mcp-flow-bundle-');
    const archivePath = path.join(dir, 'flow-fixture.tgz');
    const originalFetch = globalThis.fetch;
    try {
      await writeBundle(sourceDir, { descriptor: { metadata: { product: 'demo-product', version: '2.0.0' } } });
      const bytes = await archiveBundle(sourceDir, archivePath);
      const md5 = crypto.createHash('md5').update(bytes).digest('hex');
      let artifactHost = 'bucket.oss-cn-hangzhou.aliyuncs.com';

      const runs = [{ pipelineRunId: 7, status: 'SUCCESS' }, { pipelineRunId: 8, status: 'FAIL' }];
      const detail = () => ({
        status: 'SUCCESS',
        startTime: 1000,
        CI_COMMIT_SHA: 'd'.repeat(40),
        CI_COMMIT_ID: 'ddddddd',
        CI_COMMIT_REF_NAME: 'main',
        artifacts: [{ artifact: 'deploy-bundle', version: '2.0.0', downloadUrl: `https://${artifactHost}/bundle.tgz?signature=x`, md5, fileSize: bytes.length }]
      });
      globalThis.fetch = async (url) => {
        const text = String(url);
        if (text.includes('/runs?')) {
          return new Response(JSON.stringify(runs), { status: 200, headers: { 'content-type': 'application/json' } });
        }
        if (text.endsWith('/runs/7')) {
          return new Response(JSON.stringify(detail()), { status: 200, headers: { 'content-type': 'application/json' } });
        }
        if (text.startsWith(`https://${artifactHost}/`)) {
          return new Response(new Uint8Array(bytes), { status: 200, headers: { 'content-length': String(bytes.length) } });
        }
        throw new Error(`意外的请求：${text}`);
      };

      await call('deploy_artifact_source_save', {
        name: '云效主源', type: 'yunxiaoFlow', orgId: 'org', pipelineId: 'pipe'
      });

      const releases = await call('deploy_artifact_releases', {
        source: '云效主源', token: 'one-time-token'
      });
      assert.equal(releases.result.isError, undefined);
      assert.equal(releases.body.items.length, 1);
      assert.equal(releases.body.items[0].runId, 7);
      assert.equal(releases.body.items[0].version, '2.0.0');
      assert.equal(releases.body.items[0].commitShort, 'ddddddd');
      await assert.rejects(fs.stat(path.join(dir, 'artifact-releases-cache.json')), { code: 'ENOENT' });

      const downloaded = await call('deploy_artifact_download', {
        source: '云效主源', version: '2.0.0', token: 'one-time-token'
      });
      assert.equal(downloaded.result.isError, undefined);
      assert.equal(downloaded.body.md5, md5);
      assert.equal(downloaded.body.version, '2.0.0');
      await fs.stat(path.join(dir, 'artifact-releases-cache.json'));
      const cacheText = await fs.readFile(path.join(dir, 'artifact-releases-cache.json'), 'utf8');
      assert.equal(cacheText.includes('signature=x'), false);

      artifactHost = 'evil.example.test';
      const denied = await call('deploy_artifact_download', {
        source: '云效主源', version: '2.0.0', token: 'one-time-token'
      });
      assert.equal(denied.body.code, 'ARTIFACT_URL_HOST_DENIED');
    } finally {
      globalThis.fetch = originalFetch;
      await fs.rm(sourceDir, { recursive: true, force: true });
    }
  });
});
