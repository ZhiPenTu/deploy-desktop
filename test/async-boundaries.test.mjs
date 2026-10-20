import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRequestScope } from '../src/shared/async/requestScope.js';
import { createProgressSubscription } from '../src/shared/async/progressSubscription.js';

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('a newer request, target change or disposed view prevents stale state updates', async () => {
  const scope = createRequestScope();
  const old = deferred();
  let displayed;
  const first = scope.begin();
  const pending = old.promise.then((value) => { if (scope.isCurrent(first)) displayed = value; });
  const second = scope.begin();
  if (scope.isCurrent(second)) displayed = 'new target';
  old.resolve('old target');
  await pending;
  assert.equal(displayed, 'new target');
  scope.invalidate();
  assert.equal(scope.isCurrent(second), false);
  const third = scope.begin();
  scope.dispose();
  assert.equal(scope.isCurrent(third), false);
  assert.equal(scope.isCurrent(scope.begin()), false);
});

test('late registration is released once and cannot send progress after disposal', async () => {
  const pending = deferred();
  let callback;
  let stopped = 0;
  const events = [];
  const subscription = createProgressSubscription((handler) => {
    callback = handler;
    return pending.promise;
  }, (event) => events.push(event));
  await Promise.resolve();
  callback('before');
  subscription.dispose();
  callback('after');
  pending.resolve(() => { stopped += 1; });
  await subscription.ready;
  subscription.dispose();
  assert.equal(stopped, 1);
  assert.deepEqual(events, ['before']);
});

test('active listeners are released and registration failures are handled without rejecting readiness', async () => {
  let stopped = 0;
  const errors = [];
  const listener = createProgressSubscription(() => () => { stopped += 1; }, () => {});
  await listener.ready;
  listener.dispose();
  listener.dispose();
  await Promise.resolve();
  assert.equal(stopped, 1);
  const failed = createProgressSubscription(() => Promise.reject(new Error('unavailable')), () => {}, (error) => errors.push(error.message));
  await failed.ready;
  assert.deepEqual(errors, ['unavailable']);
});
