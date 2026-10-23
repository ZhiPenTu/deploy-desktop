import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import nodeTest from 'node:test';
// These operations execute on a Unix SSH target; Windows runs the portable executor tests.
const test = (name, fn) => nodeTest(name, { skip: process.platform === 'win32' }, fn);
import { run } from '../src/exec.mjs';
import { bashScript, stdinProgram, transferScript, backupScript, applyScript } from '../src/shell.mjs';

const local = (script, options = {}) => run('bash', ['--noprofile', '--norc', '-s'], { input: stdinProgram(script), timeoutMs: 5000, ...options });
async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'deploy-shell-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return root;
}
async function prepare(target) {
  const result = await local(transferScript('prepare', target));
  assert.equal(result.code, 0, result.stderr);
  return result.stdout.trim();
}

test('Bash 参数按字面传递：引号、换行、命令替换和空值', async () => {
  const values = ['', "a'b", '$(exit 7)', '`exit 8`', 'a\nb', '-n'];
  const result = await local(bashScript("printf '%s\\0' \"$@\"", values));
  assert.equal(result.code, 0, result.stderr);
  assert.deepEqual(result.stdout.split('\0').slice(0, -1), values);
  assert.throws(() => bashScript('true', ['bad\0value']), /无效字符/);
});

test('脚本中的读输入命令不会吞掉后续脚本', async () => {
  const result = await local('cat >/dev/null\nprintf done');
  assert.equal(result.stdout, 'done');
});

test('目录成功落位后清理旧内容，特殊字符路径不执行命令', async (t) => {
  const root = await fixture(t);
  const target = path.join(root, "site ' $(touch injected) `literal`");
  await fs.mkdir(target);
  await fs.writeFile(path.join(target, 'old'), 'old');
  const stage = await prepare(target);
  await fs.mkdir(path.join(stage, 'payload'));
  await fs.writeFile(path.join(stage, 'payload/new'), 'new');
  const result = await local(transferScript('commit', target, stage));
  assert.equal(result.code, 0, result.stderr);
  assert.deepEqual(await fs.readdir(target), ['new']);
  assert.equal((await local(transferScript('cleanup', target, stage))).code, 0);
  assert.equal(await fs.stat(stage).catch(() => null), null);
});

test('落位失败恢复旧目录；恢复失败保留可恢复副本', async (t) => {
  const root = await fixture(t);
  const bin = path.join(root, 'bin');
  await fs.mkdir(bin);
  const realMv = (await run('which', ['mv'])).stdout.trim();
  await fs.writeFile(path.join(bin, 'mv'), `#!/bin/bash\ncase "$2" in */payload) exit 9;; */previous) [ "\${FAIL_RESTORE:-}" != yes ] || exit 8;; esac\nexec '${realMv}' "$@"\n`, { mode: 0o755 });
  for (const failRestore of [false, true]) {
    const target = path.join(root, failRestore ? 'unrestored' : 'restored');
    await fs.mkdir(target);
    await fs.writeFile(path.join(target, 'old'), 'old');
    const stage = await prepare(target);
    await fs.mkdir(path.join(stage, 'payload'));
    const result = await local(transferScript('commit', target, stage), { env: { PATH: `${bin}:${process.env.PATH}`, FAIL_RESTORE: failRestore ? 'yes' : '' } });
    assert.notEqual(result.code, 0);
    assert.equal(await fs.readFile(path.join(failRestore ? `${stage}/previous` : target, 'old'), 'utf8'), 'old');
    const cleanup = await local(transferScript('cleanup', target, stage));
    assert.equal(cleanup.code, failRestore ? 5 : 0, cleanup.stderr);
  }
});

test('已有替换锁与无效暂存路径不能改动部署', async (t) => {
  const root = await fixture(t);
  const target = path.join(root, 'app');
  const stage = await prepare(target);
  await fs.writeFile(path.join(stage, 'payload'), 'new');
  await fs.writeFile(target, 'old');
  await fs.mkdir(`${target}.deploy-lock`);
  assert.equal((await local(transferScript('commit', target, stage))).code, 3);
  assert.equal((await local(transferScript('cleanup', target, root))).code, 2);
  assert.equal((await local(transferScript('cleanup', target, `${stage}/../app`))).code, 2);
  assert.equal((await local(transferScript('cleanup', target, stage))).code, 3);
  assert.equal(await fs.readFile(target, 'utf8'), 'old');
});

test('损坏的压缩包不会覆盖已有目录', async (t) => {
  const root = await fixture(t);
  const target = path.join(root, 'app');
  await fs.mkdir(target);
  await fs.writeFile(path.join(target, 'old'), 'old');
  const stage = await prepare(target);
  const result = await run('bash', ['-c', transferScript('receive-dir', target, stage)], { input: 'not a tar archive' });
  assert.notEqual(result.code, 0);
  assert.equal(await fs.readFile(path.join(target, 'old'), 'utf8'), 'old');
});

test('备份保留完整安装目录层级，名称唯一，拒绝越界路径', async (t) => {
  const root = await fixture(t);
  const targets = ['config/app.yml', "deploy/config/app'$(x).yml"].map((name) => path.join(root, name));
  for (const [i, file] of targets.entries()) { await fs.mkdir(path.dirname(file), { recursive: true }); await fs.writeFile(file, String(i)); }
  const backups = [];
  for (let i = 0; i < 2; i++) {
    const result = await local(backupScript(root, targets));
    assert.equal(result.code, 0, result.stderr);
    const backup = result.stdout.trim().slice('BACKUP_DIR='.length);
    backups.push(backup);
    for (const [i, file] of targets.entries()) assert.equal(await fs.readFile(path.join(backup, path.relative(root, file)), 'utf8'), String(i));
  }
  assert.notEqual(backups[0], backups[1]);
  assert.equal((await local(backupScript(root, [`${root}/../outside`]))).code, 2);
});

test('Compose 使用实际容器 ID 并保持离线更新参数；缺失服务失败', async (t) => {
  const root = await fixture(t);
  const bin = path.join(root, 'bin');
  await fs.mkdir(bin);
  await fs.writeFile(path.join(root, "compose ' $(literal).yml"), 'services: {}');
  await fs.writeFile(path.join(bin, 'docker'), `#!/bin/bash
printf '%s\\n' "$*" >> "$CALLS"
case "$*" in
 'compose version') exit 0;;
 *'ps -a -q'*) [ "\${MISSING:-}" = yes ] || echo actual-container-id;;
 'inspect --format {{.State.Status}} actual-container-id') echo running;;
 inspect*) echo none;;
esac
`, { mode: 0o755 });
  const options = { env: { PATH: `${bin}:${process.env.PATH}`, CALLS: path.join(root, 'calls') } };
  const script = applyScript({ deployDir: root, compose: "compose ' $(literal).yml", config: true, restart: ['api'], wait: ['api'] });
  const result = await local(script, options);
  assert.equal(result.code, 0, result.stderr);
  const calls = await fs.readFile(options.env.CALLS, 'utf8');
  assert.match(calls, /up -d --pull never --no-build --remove-orphans/);
  assert.match(calls, /inspect .*actual-container-id/);
  const missing = await local(script, { env: { ...options.env, MISSING: 'yes' } });
  assert.equal(missing.code, 4);
  assert.match(missing.stderr, /服务 api 未创建/);
  assert.equal((await local(applyScript({ deployDir: root, compose: "compose ' $(literal).yml", config: false, restart: [], wait: [] }), options)).code, 0);
});
