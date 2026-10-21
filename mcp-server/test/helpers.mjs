import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

export const DIGEST_A = 'a'.repeat(64);
export const DIGEST_B = 'b'.repeat(64);

export async function makeTempDir(prefix = 'deploy-mcp-test-') {
  return fs.mkdtemp(path.join(os.tmpdir(), prefix));
}

export function makeDescriptor(overrides = {}) {
  const descriptor = {
    apiVersion: overrides.apiVersion || 'deploy.desktop/bundle/v2',
    kind: overrides.kind || 'DeploymentBundle',
    metadata: {
      product: 'demo-product',
      displayName: '演示产品',
      version: '1.0.0',
      buildNumber: '42',
      commit: 'c'.repeat(40),
      commitShort: 'ccccccc',
      branch: 'main',
      builtAt: '2026-10-20T00:00:00Z',
      minToolVersion: '',
      ...(overrides.metadata || {})
    },
    runtime: {
      type: 'compose',
      compose: 'deploy/docker-compose.yml',
      variant: '',
      containerPrefix: 'demo',
      services: [
        { key: 'api', label: '接口服务', port: 8000, protocol: 'TCP' },
        { key: 'web', label: '网页服务', port: 18080, protocol: 'TCP' },
        { key: 'db', label: '数据库', port: 2881, protocol: 'TCP' }
      ],
      database: { serviceKey: 'db', name: 'demo_db', client: 'obclient', port: 2881 },
      ...(overrides.runtime || {})
    },
    components: overrides.components ?? [
      { key: 'server', label: '服务端', artifacts: ['server-bin', 'server-config'], services: ['api'] },
      { key: 'site', label: '网页资源', artifacts: ['web-dist'], services: ['web'] },
      { key: 'config', label: '部署配置', artifacts: ['compose', 'db-config'], services: [] }
    ],
    artifacts: overrides.artifacts ?? [
      { key: 'server-bin', label: '服务程序', source: 'app/bin', root: 'install', path: 'bin', type: 'dir' },
      { key: 'server-config', label: '服务配置', source: 'app/config/app.yml', root: 'install', path: 'config/app.yml', type: 'file' },
      { key: 'web-dist', label: '网页文件', source: 'app/web', root: 'deploy', path: 'site', type: 'dir' },
      { key: 'compose', label: 'Compose 清单', source: 'deploy/docker-compose.yml', root: 'deploy', path: 'docker-compose.yml', type: 'file' },
      { key: 'db-config', label: '数据库初始化', source: 'deploy/db', root: 'deploy', path: 'db', type: 'dir' }
    ],
    images: overrides.images ?? [
      { name: 'db', ref: `demo/db@sha256:${DIGEST_A}`, role: 'base', required: true, bundleFile: 'images/db.tar' }
    ],
    plan: {
      steps: [
        { key: 'verify', title: '检查部署包', hint: '确认版本与文件完整性' },
        { key: 'apply', title: '应用更新', hint: '替换文件并重启服务' }
      ]
    },
    files: []
  };
  return descriptor;
}

export function bundleFiles() {
  return {
    'app/bin/server': '#!/bin/sh\n',
    'app/config/app.yml': 'server:\n  port: 8000\n',
    'app/web/index.html': '<html></html>\n',
    'deploy/docker-compose.yml': 'services:\n  api:\n    image: demo/api@sha256:' + DIGEST_A + '\n',
    'deploy/db/init.sql': 'SELECT 1;\n',
    'images/db.tar': 'tar-bytes'
  };
}

export async function writeBundle(root, options = {}) {
  const descriptor = makeDescriptor(options.descriptor || {});
  const contents = { ...bundleFiles(), ...(options.contents || {}) };
  for (const [relative, content] of Object.entries(contents)) {
    const target = path.join(root, relative);
    await fs.mkdir(path.dirname(target), { recursive: true });
    await fs.writeFile(target, content);
  }
  if (options.mutate) await options.mutate({ root, descriptor, contents });
  descriptor.files = options.files || await Promise.all(Object.entries(contents).map(async ([relative, content]) => ({
    path: relative.split(path.sep).join('/'),
    size: Buffer.byteLength(content),
    sha256: crypto.createHash('sha256').update(content).digest('hex')
  })));
  await fs.writeFile(path.join(root, 'bundle.json'), `${JSON.stringify(descriptor, null, 2)}\n`);
  return { descriptor, contents };
}

export function makeProject(overrides = {}) {
  return {
    id: 'proj-1',
    name: '演示项目',
    product: 'demo-product',
    hostId: 'host-1',
    installDir: '/opt/demo',
    sourceId: 'src-1',
    lastVersion: '1.0.0',
    lastBundleDir: '/tmp/bundle',
    ...overrides
  };
}

export function makeManifest({ metadata = {}, spec = {}, ...rest } = {}) {
  return {
    apiVersion: 'deploy.desktop/v1',
    kind: 'ImageBundle',
    metadata: {
      name: 'demo-stack',
      version: '1.0.0',
      ...metadata
    },
    spec: {
      registry: 'registry.example.com',
      namespace: 'team',
      platform: 'linux/amd64',
      images: [
        { name: 'backend', repository: 'backend', digest: `sha256:${DIGEST_A}`, sizeBytes: 1024 },
        { name: 'frontend', repository: 'frontend', digest: `sha256:${DIGEST_B}`, sizeBytes: 2048 }
      ],
      ...spec
    },
    ...rest
  };
}
