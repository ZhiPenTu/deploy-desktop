import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

export const DIGEST_A = 'a'.repeat(64);
export const DIGEST_B = 'b'.repeat(64);

export async function makeTempDir(prefix = 'deploy-mcp-test-') {
  return fs.mkdtemp(path.join(os.tmpdir(), prefix));
}

export function makeProfile(overrides = {}) {
  return {
    name: '测试部署配置',
    containerPrefix: 'demo',
    bundleDir: '/opt/deploy/bundle',
    deployDir: '/opt/deploy',
    installDir: '/opt/deploy',
    backendArtifact: 'demo-server.jar',
    database: {
      serviceKey: 'oceanbase',
      name: 'demo_db',
      client: 'obclient',
      port: 2881
    },
    containers: [
      { serviceKey: 'oceanbase', label: '数据库', port: 2881, protocol: 'TCP' },
      { serviceKey: 'backend-java', label: '后端', port: 8000, protocol: 'TCP' },
      { serviceKey: 'gateway-shard-1', label: '网关', port: null, protocol: 'TCP' },
      { serviceKey: 'frontend', label: '前端', port: 18080, protocol: 'TCP' }
    ],
    images: [{ file: 'oceanbase.tar', imageRef: 'oceanbase/oceanbase-ce:4.4.2-lts' }],
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
