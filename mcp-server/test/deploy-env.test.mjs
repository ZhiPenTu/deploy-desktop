import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { assertComposeEnv, composeRequiredEnv } from '../src/deploy.mjs';
import { DeployError } from '../src/errors.mjs';
import { makeTempDir } from './helpers.mjs';

test('composeRequiredEnv 只统计 compose 会直接报错的变量', () => {
  const content = [
    'services:',
    '  api:',
    '    environment:',
    '      A: ${DEMO_PLAIN}',
    '      B: ${DEMO_DEFAULT:-fallback}',
    '      C: ${DEMO_DASH-default}',
    '      D: ${DEMO_ALT:+other}',
    '      E: ${DEMO_CHECK:?必须填写}',
    '      F: ${DEMO_CHECK?必须填写}',
    '      G: $${DEMO_ESCAPED}'
  ].join('\n');
  assert.deepEqual(composeRequiredEnv(content).sort(), ['DEMO_CHECK']);
});

test('缺少 Compose 变量时返回 ENV_MISSING 并指到 .env', async () => {
  const dir = await makeTempDir();
  const compose = path.join(dir, 'docker-compose.yml');
  await fs.writeFile(compose, 'services:\n  api:\n    environment:\n      PW: ${MCP_TEST_MISSING_PASSWORD:?必填}\n');
  await assert.rejects(() => assertComposeEnv(dir, compose), (error) => {
    assert.ok(error instanceof DeployError);
    assert.equal(error.code, 'ENV_MISSING');
    assert.match(error.message, /MCP_TEST_MISSING_PASSWORD/);
    assert.match(error.hint, /\.env/);
    return true;
  });
});

test('.env 提供变量后校验通过，空值仍视为缺失', async () => {
  const dir = await makeTempDir();
  const compose = path.join(dir, 'docker-compose.yml');
  await fs.writeFile(compose, 'services:\n  api:\n    environment:\n      PW: ${MCP_TEST_PRESENT_PASSWORD:?必填}\n');
  const envFile = path.join(dir, '.env');
  await fs.writeFile(envFile, '# 注释\nexport MCP_TEST_PRESENT_PASSWORD=secret\n');
  assert.deepEqual(await assertComposeEnv(dir, compose), ['MCP_TEST_PRESENT_PASSWORD']);
  await fs.writeFile(envFile, 'MCP_TEST_PRESENT_PASSWORD=\n');
  await assert.rejects(() => assertComposeEnv(dir, compose), /MCP_TEST_PRESENT_PASSWORD/);
});
