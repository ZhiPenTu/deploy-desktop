import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  isBundleDirInUse,
  localStatusText,
  releaseKey,
  shortPath,
  versionText
} from '../src/features/artifacts/domain/releases.js';

test('versionText 区分未标注版本与正常版本', () => {
  assert.equal(versionText({ version: '1.2.3' }), '1.2.3');
  assert.equal(versionText({ version: ' manual ' }), '未标注版本');
  assert.equal(versionText({ version: '   ' }), '未标注版本');
  assert.equal(versionText(null), '未标注版本');
});

test('releaseKey 在同版本不同构建之间保持唯一', () => {
  const first = { runId: 12, artifactName: 'app', version: '1.0.0' };
  assert.equal(releaseKey(first), '12-app-1.0.0');
  assert.notEqual(releaseKey(first), releaseKey({ ...first, runId: 13 }));
  assert.equal(releaseKey({}), '0--');
});

test('localStatusText 只依据本地目录判断下载状态', () => {
  assert.equal(localStatusText({ localDir: '/opt/bundle' }), '已下载');
  assert.equal(localStatusText({ localDir: '' }), '未下载');
  assert.equal(localStatusText(undefined), '未下载');
});

test('shortPath 只压缩超过两级的目录', () => {
  assert.equal(shortPath('/opt/app'), '/opt/app');
  assert.equal(shortPath('/srv/data/app/bundles/1.0.0'), '.../bundles/1.0.0');
  assert.equal(shortPath(''), '');
});

test('isBundleDirInUse 不会把前缀相同的相邻目录当成包含关系', () => {
  assert.equal(isBundleDirInUse('/opt/app', '/opt/app'), true);
  assert.equal(isBundleDirInUse('/opt/app/releases/1.0.0', '/opt/app'), true);
  // 原样比较：末尾分隔符不归一化，保持既有判定结果。
  assert.equal(isBundleDirInUse('/opt/app', '/opt/app/'), false);
  assert.equal(isBundleDirInUse('/opt/app2', '/opt/app'), false);
  assert.equal(isBundleDirInUse('', '/opt/app'), false);
  assert.equal(isBundleDirInUse('/opt/app', ''), false);
});
