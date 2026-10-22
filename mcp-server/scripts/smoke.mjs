import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const serverDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const projectRoot = path.resolve(serverDir, '..');
const configDir = await fs.mkdtemp(path.join(os.tmpdir(), 'deploy-mcp-smoke-config-'));
const bundleDir = await fs.mkdtemp(path.join(os.tmpdir(), 'deploy-mcp-smoke-bundle-'));

function parse(result) {
  return JSON.parse(result.content[0].text);
}

async function main() {
  const contents = {
    'app/web/index.html': '<html></html>\n',
    'deploy/docker-compose.yml': 'services:\n  web:\n    image: demo/web:1.0.0\n',
    'images/demo.tar': Buffer.alloc(64)
  };
  for (const [relative, content] of Object.entries(contents)) {
    const target = path.join(bundleDir, relative);
    await fs.mkdir(path.dirname(target), { recursive: true });
    await fs.writeFile(target, content);
  }
  await fs.writeFile(path.join(bundleDir, 'bundle.json'), JSON.stringify({
    apiVersion: 'deploy.desktop/bundle/v2',
    kind: 'DeploymentBundle',
    metadata: { product: 'smoke-product', displayName: '冒烟测试', version: '1.0.0' },
    runtime: {
      type: 'compose',
      compose: 'deploy/docker-compose.yml',
      containerPrefix: 'smoke',
      services: [{ key: 'web', label: '网页服务', port: 18080, protocol: 'TCP' }]
    },
    components: [{ key: 'site', label: '网页资源', artifacts: ['web', 'compose'], services: ['web'] }],
    artifacts: [
      { key: 'web', label: '网页文件', source: 'app/web', root: 'deploy', path: 'site', type: 'dir' },
      { key: 'compose', label: 'Compose 清单', source: 'deploy/docker-compose.yml', root: 'deploy', path: 'docker-compose.yml', type: 'file' }
    ],
    images: [{ name: 'web', ref: 'demo/web@sha256:' + 'a'.repeat(64), role: 'app', required: true, bundleFile: 'images/demo.tar' }],
    plan: { steps: [{ key: 'verify', title: '检查部署包' }] },
    files: await Promise.all(Object.entries(contents).map(async ([relative, content]) => {
      const { createHash } = await import('node:crypto');
      const bytes = Buffer.isBuffer(content) ? content : Buffer.from(content);
      return { path: relative, size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
    }))
  }, null, 2));

  await fs.writeFile(path.join(configDir, 'projects.json'), JSON.stringify({
    schemaVersion: 1,
    activeProjectId: 'proj-smoke',
    projects: [{
      id: 'proj-smoke',
      name: '冒烟项目',
      product: 'smoke-product',
      hostId: 'host-smoke',
      installDir: '/opt/smoke',
      sourceId: 'src-smoke',
      lastVersion: '1.0.0',
      lastBundleDir: bundleDir
    }]
  }));

  const env = Object.fromEntries(Object.entries(process.env).filter(([, value]) => value !== undefined));
  env.DEPLOY_DESKTOP_CONFIG_DIR = configDir;
  env.DEPLOY_DESKTOP_APP_CONFIG_DIR = configDir;
  env.DEPLOY_DESKTOP_ROOT = projectRoot;

  const transport = new StdioClientTransport({
    command: process.execPath,
    args: [path.join(serverDir, 'index.mjs')],
    cwd: serverDir,
    env
  });
  const client = new Client({ name: 'deploy-desktop-smoke', version: '1.0.0' });
  await client.connect(transport);
  try {
    const tools = await client.listTools();
    assert.equal(tools.tools.length, 51);
    console.log(`tools: ${tools.tools.length}`);

    const profile = parse(await client.callTool({ name: 'deploy_profile_get', arguments: {} }));
    assert.equal(profile.profile.containerPrefix, 'smoke');
    console.log(`profile: ${profile.profile.containerPrefix} (${profile.profile.containers.length} containers)`);

    const projects = parse(await client.callTool({ name: 'deploy_project_list', arguments: {} }));
    assert.equal(projects.projects.length, 1);
    assert.equal(projects.activeProjectId, 'proj-smoke');
    console.log(`projects: ${projects.projects.length}`);

    const source = parse(await client.callTool({
      name: 'deploy_artifact_source_save',
      arguments: { name: 'smoke-source', type: 'url', version: '1.0.0', downloadUrl: 'https://example.com/bundle.tgz' }
    }));
    assert.equal(source.source.hasToken, false);
    const sources = parse(await client.callTool({ name: 'deploy_artifact_source_list', arguments: {} }));
    assert.equal(sources.sources.length, 1);
    console.log(`artifact sources: ${sources.sources.length}`);

    const registry = parse(await client.callTool({
      name: 'deploy_registry_save',
      arguments: { name: 'smoke', url: 'https://registry.example.com/', namespace: 'smoke-ns', isDefault: true }
    }));
    assert.equal(registry.registry.url, 'registry.example.com');
    console.log(`registry: ${registry.registry.url}/${registry.registry.namespace}`);

    const manifest = parse(await client.callTool({
      name: 'deploy_manifest_load',
      arguments: { source: path.join(projectRoot, 'examples', 'manifest.example.yaml'), registry: 'smoke' }
    }));
    assert.equal(manifest.registry, 'registry.example.com');
    assert.equal(manifest.namespace, 'smoke-ns');
    console.log(`manifest: ${manifest.images.length} images`);

    const scan = parse(await client.callTool({ name: 'deploy_bundle_scan', arguments: { dir: bundleDir } }));
    assert.equal(scan.files.length, 1);
    assert.equal(scan.totalBytes, 64);
    console.log(`bundle: ${scan.files.length} file, ${scan.totalBytes} bytes`);

    const denied = await client.callTool({ name: 'deploy_db_import', arguments: { target: 'local', sqlFile: '/tmp/not-run.sql' } });
    assert.equal(denied.isError, true);
    assert.equal(parse(denied).code, 'CONFIRM_REQUIRED');
    console.log('confirm boundary: CONFIRM_REQUIRED');
  } finally {
    await client.close();
  }
}

try {
  await main();
  console.log('smoke: OK');
} finally {
  await fs.rm(configDir, { recursive: true, force: true });
  await fs.rm(bundleDir, { recursive: true, force: true });
}
