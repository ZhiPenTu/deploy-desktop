import assert from 'node:assert/strict';
import { test } from 'node:test';
import { errorText } from '../src/shared/errors.js';

test('错误文案取异常消息，不带 Error 前缀', () => {
  assert.equal(errorText(new Error('目标主机连接失败')), '目标主机连接失败');
  assert.equal(errorText('目标主机连接失败'), '目标主机连接失败');
  assert.equal(errorText({ message: '命令执行失败' }), '命令执行失败');
  assert.equal(errorText({ message: '  目录不可写  ' }), '目录不可写');
});

test('缺少消息时使用可操作的中文提示', () => {
  assert.equal(errorText(null), '操作失败，请稍后重试。');
  assert.equal(errorText(undefined), '操作失败，请稍后重试。');
  assert.equal(errorText(new Error('')), '操作失败，请稍后重试。');
  assert.equal(errorText('   '), '操作失败，请稍后重试。');
  assert.equal(errorText({}), '操作失败，请稍后重试。');
  assert.equal(errorText(null, '读取失败，请重试。'), '读取失败，请重试。');
});
