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
  await fs.mkdir(path.join(bundleDir, 'images'), { recursive: true });
  await fs.writeFile(path.join(bundleDir, 'images', 'demo.tar'), Buffer.alloc(64));

  const env = Object.fromEntries(Object.entries(process.env).filter(([, value]) => value !== undefined));
  env.DEPLOY_DESKTOP_CONFIG_DIR = configDir;
  env.DEPLOY_DESKTOP_ROOT = projectRoot;
  env.DEPLOY_DESKTOP_PROFILE = path.join(projectRoot, 'profile.example.json');

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
    assert.equal(tools.tools.length, 39);
    console.log(`tools: ${tools.tools.length}`);

    const profile = parse(await client.callTool({ name: 'deploy_profile_get', arguments: {} }));
    assert.equal(profile.profile.containerPrefix, 'demo-stack');
    console.log(`profile: ${profile.profile.containerPrefix} (${profile.profile.containers.length} containers)`);

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
