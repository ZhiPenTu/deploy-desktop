import assert from 'node:assert/strict';
import { test } from 'node:test';
import { wrapTool, progressReporter } from '../src/tools/runtime.mjs';
import { DeployError } from '../src/errors.mjs';

test('tool boundary preserves results and structured failure codes', async () => {
  const args = { confirm: false };
  const extra = { requestId: 'request' };
  const wrapped = wrapTool((actualArgs, actualExtra) => {
    assert.equal(actualArgs, args);
    assert.equal(actualExtra, extra);
    throw new DeployError('需要确认', { code: 'CONFIRM_REQUIRED', hint: '确认目标', details: { stage: 'preflight' } });
  });
  const failed = await wrapped(args, extra);
  assert.equal(failed.isError, true);
  assert.deepEqual(JSON.parse(failed.content[0].text), { error: '需要确认', code: 'CONFIRM_REQUIRED', hint: '确认目标', details: { stage: 'preflight' } });
  assert.equal((await wrapTool(() => 'output')({}, {})).content[0].text, 'output');
  assert.deepEqual(JSON.parse((await wrapTool(() => ({ success: false }))({}, {})).content[0].text), { success: false });
});

test('missing or failing progress delivery does not fail the operation', async () => {
  await progressReporter({})(1, 2, '准备');
  const notifications = [];
  const progress = progressReporter({ _meta: { progressToken: 0 }, sendNotification: async (value) => {
    notifications.push(value);
    throw new Error('client disconnected');
  } });
  await progress(1, 2, '准备');
  assert.deepEqual(notifications, [{ method: 'notifications/progress', params: { progressToken: 0, progress: 1, total: 2, message: '准备' } }]);
});
