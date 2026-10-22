import assert from 'node:assert/strict';
import path from 'node:path';
import test from 'node:test';
import { PROJECT_ROOT, saveConfig } from '../src/config.mjs';
import { registerTools } from '../src/tools.mjs';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs/promises';
import { makeTempDir, writeBundle } from './helpers.mjs';

const ORIGINAL_CONFIG_DIR = process.env.DEPLOY_DESKTOP_CONFIG_DIR;
const ORIGINAL_CONFIG_FILE = process.env.DEPLOY_DESKTOP_CONFIG_FILE;

function restoreEnv() {
  if (ORIGINAL_CONFIG_DIR === undefined) delete process.env.DEPLOY_DESKTOP_CONFIG_DIR;
  else process.env.DEPLOY_DESKTOP_CONFIG_DIR = ORIGINAL_CONFIG_DIR;
  if (ORIGINAL_CONFIG_FILE === undefined) delete process.env.DEPLOY_DESKTOP_CONFIG_FILE;
  else process.env.DEPLOY_DESKTOP_CONFIG_FILE = ORIGINAL_CONFIG_FILE;
}

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

function parseResult(result) {
  return JSON.parse(result.content[0].text);
}

const tools = captureTools();

test('注册完整工具集且名称唯一', () => {
  assert.equal(tools.size, 51);
  for (const name of [
    'deploy_project_list',
    'deploy_project_create',
    'deploy_project_update',
    'deploy_project_remove',
    'deploy_project_activate',
    'deploy_project_bind_bundle',
    'deploy_artifact_source_list',
    'deploy_artifact_source_save',
    'deploy_artifact_source_remove',
    'deploy_artifact_releases',
    'deploy_artifact_download',
    'deploy_registry_save',
    'deploy_registry_probe',
    'deploy_manifest_load',
    'deploy_image_pull',
    'deploy_image_save',
    'deploy_bundle_upload',
    'deploy_bundle_deploy',
    'deploy_compose',
    'deploy_inspect',
    'deploy_db_import',
    'deploy_update_remote',
    'deploy_app_update_check'
  ]) {
    assert.equal(tools.has(name), true, `缺少工具 ${name}`);
  }
});

test('破坏性工具标记 destructiveHint', () => {
  for (const name of [
    'deploy_project_remove',
    'deploy_artifact_source_remove',
    'deploy_host_remove',
    'deploy_registry_remove',
    'deploy_image_remove',
    'deploy_bundle_remove',
    'deploy_bundle_deploy',
    'deploy_compose',
    'deploy_remote_run',
    'deploy_db_import',
    'deploy_docker_install',
    'deploy_update_remote'
  ]) {
    assert.equal(tools.get(name).definition.annotations?.destructiveHint, true, `${name} 应标记为破坏性操作`);
  }
});

test('deploy_profile_get 读取当前项目绑定的交付包', async () => {
  const configDir = await makeTempDir();
  const bundleDir = await makeTempDir();
  await writeBundle(bundleDir);
  const previousDir = process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
  process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = configDir;
  try {
    await fs.writeFile(path.join(configDir, 'projects.json'), JSON.stringify({
      schemaVersion: 1,
      activeProjectId: 'proj-1',
      projects: [{
        id: 'proj-1',
        name: '演示项目',
        product: 'demo-product',
        hostId: 'host-1',
        installDir: '/opt/demo',
        sourceId: 'src-1',
        lastVersion: '1.0.0',
        lastBundleDir: bundleDir
      }]
    }));
    const result = await tools.get('deploy_profile_get').handler({}, {});
    assert.equal(result.isError, undefined);
    const payload = parseResult(result);
    assert.equal(payload.profile.containerPrefix, 'demo');
    assert.equal(payload.project.name, '演示项目');
    assert.equal(payload.bundle.components.length, 3);
  } finally {
    if (previousDir === undefined) delete process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
    else process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = previousDir;
  }
});

test('deploy_profile_get 在没有项目时返回明确错误', async () => {
  const configDir = await makeTempDir();
  const previousDir = process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
  process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = configDir;
  try {
    const result = await tools.get('deploy_profile_get').handler({}, {});
    assert.equal(result.isError, true);
    assert.match(result.content[0].text, /尚未创建项目/);
  } finally {
    if (previousDir === undefined) delete process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
    else process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR = previousDir;
  }
});

test('清单工具把已保存的仓库配置名解析为真实地址', async () => {
  process.env.DEPLOY_DESKTOP_CONFIG_DIR = await makeTempDir();
  delete process.env.DEPLOY_DESKTOP_CONFIG_FILE;
  try {
    await saveConfig({
      registries: {
        myacr: {
          name: 'myacr',
          url: 'registry.test.local',
          namespace: 'saved-ns',
          username: '',
          passwordEnv: '',
          insecure: false,
          isDefault: true
        }
      }
    });
    const result = await tools.get('deploy_manifest_load').handler({
      source: path.join(PROJECT_ROOT, 'examples', 'manifest.example.yaml'),
      registry: 'myacr'
    }, {});
    assert.equal(result.isError, undefined);
    const payload = parseResult(result);
    assert.equal(payload.registry, 'registry.test.local');
    assert.equal(payload.namespace, 'saved-ns');
    assert.match(payload.images[0].reference, /^registry\.test\.local\/saved-ns\//);
  } finally {
    restoreEnv();
  }
});

test('数据库导入与一键部署缺少 confirm 时拒绝执行', async () => {
  for (const [name, args] of [
    ['deploy_db_import', { target: 'local', sqlFile: '/tmp/not-run.sql' }],
    ['deploy_bundle_deploy', { target: 'local', bundleDir: '/tmp/not-run' }]
  ]) {
    const result = await tools.get(name).handler(args, {});
    assert.equal(result.isError, true);
    assert.equal(parseResult(result).code, 'CONFIRM_REQUIRED');
  }
});

test('删除镜像缺少 confirm 时拒绝执行', async () => {
  const result = await tools.get('deploy_image_remove').handler({
    target: 'local',
    references: ['demo:latest']
  }, {});
  assert.equal(result.isError, true);
  assert.equal(parseResult(result).code, 'CONFIRM_REQUIRED');
});
