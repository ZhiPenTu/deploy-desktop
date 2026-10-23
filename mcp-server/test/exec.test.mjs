import assert from 'node:assert/strict';
import test from 'node:test';
import { run, runChecked } from '../src/exec.mjs';
const node = (code) => ['-e', code];

test('进程在写入大输入时并行读取输出，避免管道死锁', async () => {
  const result = await run(process.execPath, node("process.stderr.write('e'.repeat(200000)); process.stdin.pipe(process.stdout)"), { input: Buffer.alloc(200000, 'x'), timeoutMs: 5000 });
  assert.equal(result.code, 0);
  assert.equal(result.stdout.length, 200000);
  assert.equal(result.stderr.length, 200000);
});

test('远端提前退出时保留退出码，不触发未处理的 EPIPE', async () => {
  const result = await run(process.execPath, node("process.stderr.write('denied'); process.exit(23)"), { input: Buffer.alloc(5_000_000), timeoutMs: 5000 });
  assert.equal(result.code, 23);
  assert.equal(result.stderr, 'denied');
});

test('生产者失败时，即使接收者成功也不能报告上传成功', async () => {
  await assert.rejects(run(process.execPath, node('process.stdin.resume()'), {
    inputCommand: { command: process.execPath, args: node("process.stdout.write('partial'); process.stderr.write('bad archive'); process.exitCode=7") }, timeoutMs: 5000
  }), (error) => error.code === 'PRODUCER_FAILED' && /bad archive/.test(error.message));
});

test('超时后即使进程捕获信号并以零退出也按超时处理', async () => {
  const result = await run(process.execPath, node("process.on('SIGTERM', () => process.exit(0)); setInterval(() => {}, 100)"), { timeoutMs: 300 });
  assert.equal(result.timedOut, true);
  assert.equal(result.code, 124);
});

test('输出截断不能被 checked 调用当成完整成功结果', async () => {
  await assert.rejects(runChecked(process.execPath, node("process.stdout.write('x'.repeat(200000))"), { maxOutputBytes: 1024 }), /超过限制/);
});

test('启动失败及时返回而不遗留等待任务', async () => {
  await assert.rejects(run('/definitely/missing/deploy-executable', [], { timeoutMs: 1000 }), (error) => error.code === 'SPAWN_FAILED');
});

test('超时终止占据输出管道的子进程', { skip: process.platform === 'win32' }, async () => {
  const started = Date.now();
  const result = await run('bash', ['-c', 'sleep 30 & wait'], { timeoutMs: 100 });
  assert.equal(result.timedOut, true);
  assert.ok(Date.now() - started < 4000);
});
