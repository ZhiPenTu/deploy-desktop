import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import http from 'node:http';
import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import nodeTest from 'node:test';
import { runBashFile, versionDeployScript } from '../src/shell.mjs';
import { makeTempDir } from './helpers.mjs';

const test = (name, fn) => nodeTest(name, { skip: process.platform === 'win32' }, fn);
const worker = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../shell/version-deploy.sh'
);

const mockDocker = `#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$MOCK_DOCKER_LOG"
if [ "\${1:-}" = compose ] && [ "\${2:-}" = version ]; then
  exit 0
fi
if [ "\${1:-}" = info ]; then
  exit 0
fi
if [ "\${1:-}" = inspect ]; then
  case " $* " in
    *'.State.Status'*) printf 'running\\n' ;;
    *'.State.Health'*) printf 'healthy\\n' ;;
    *) printf 'running|healthy\\n' ;;
  esac
  exit 0
fi
if [ "\${1:-}" = volume ] && [ "\${2:-}" = rm ]; then
  rm -rf -- "$MOCK_VOLUME_DIR"
  exit 0
fi
if [ "\${1:-}" = run ]; then
  script=
  index=1
  while [ "$index" -lt "$#" ]; do
    if [ "\${!index}" = -c ]; then
      next=$((index + 1))
      script=\${!next}
      break
    fi
    index=$((index + 1))
  done
  case "$script" in
    *'command -v tar'*) exit 0 ;;
    *'tar -czf -'*) tar -cf - -C "$MOCK_VOLUME_DIR" . | gzip -c ;;
    *'tar -xzf -'*)
      if [ "\${MOCK_FAIL_RESTORE:-no}" = yes ]; then
        exit 2
      fi
      rm -rf -- "$MOCK_VOLUME_DIR"
      mkdir -p -- "$MOCK_VOLUME_DIR"
      tar -xzf - -C "$MOCK_VOLUME_DIR"
      ;;
    *) exit 0 ;;
  esac
  exit $?
fi
if [ "\${1:-}" = compose ]; then
  shift
  compose_file=
  index=0
  args=("$@")
  while [ "$index" -lt "\${#args[@]}" ]; do
    if [ "\${args[$index]}" = -f ]; then
      next=$((index + 1))
      [ -n "$compose_file" ] || compose_file=\${args[$next]}
      index=$((index + 2))
      continue
    fi
    index=$((index + 1))
  done
  case " $* " in
    *' config --services '*) printf 'frontend\\n' ;;
    *' ps -a -q '*) printf 'mock-container\\n' ;;
    *' exec -T '*|*' logs '*) ;;
    *' down '*)
      if [ "\${MOCK_FAIL_OLD_DOWN:-no}" = yes ] && [ "\${compose_file##*/}" = compose.old.yml ]; then
        printf 'old version stop failed\\n' >&2
        exit 1
      fi
      if [ "\${MOCK_FAIL_NEW_DOWN:-no}" = yes ] && [ "\${compose_file##*/}" = compose.new.yml ]; then
        printf 'stop failed\\n' >&2
        exit 1
      fi
      ;;
    *' up -d '*)
      if [ "\${compose_file##*/}" = compose.new.yml ] && [ -n "\${MOCK_NEW_UP_DELAY:-}" ]; then
        sleep "$MOCK_NEW_UP_DELAY"
      fi
      if [ "\${MOCK_FAIL_NEW_UP:-no}" = yes ] && [ -n "$compose_file" ] && [ "\${compose_file##*/}" = compose.new.yml ]; then
        printf 'new version failed\\n' >&2
        exit 1
      fi
      ;;
  esac
  exit 0
fi
exit 0
`;

const sha256sumFallback = `#!/bin/sh
if command -v shasum >/dev/null 2>&1; then
  exec shasum -a 256 "$@"
fi
openssl dgst -sha256 -r "$@" | awk '{print $1 "  " $2}'
`;

async function fixture(t) {
  const root = await fs.realpath(await makeTempDir('deploy-worker-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const bin = path.join(root, 'bin');
  const install = path.join(root, 'install');
  const deploy = path.join(install, 'deploy');
  const txn = path.join(install, '.deploy-desktop', 'transactions', 'deploy-1');
  const incoming = path.join(txn, 'incoming');
  const volume = path.join(root, 'volume');
  await fs.mkdir(bin);
  await fs.mkdir(deploy, { recursive: true });
  await fs.mkdir(path.join(incoming, 'tree', 'deploy'), { recursive: true });
  await fs.mkdir(volume);
  await fs.copyFile(
    path.resolve(path.dirname(worker), 'deployment-baseline.sh'),
    path.join(txn, 'baseline.sh')
  );
  await fs.writeFile(path.join(bin, 'docker'), mockDocker, { mode: 0o755 });
  await fs.writeFile(path.join(bin, 'sha256sum'), sha256sumFallback, { mode: 0o755 });
  if (process.platform === 'darwin') {
    await fs.writeFile(path.join(bin, 'flock'), `#!/bin/sh
exec /usr/bin/perl -e 'open my $handle, "+<&=$ARGV[0]" or die $!; flock($handle, 6) or exit 1;' "$2"
`, { mode: 0o755 });
    await fs.writeFile(path.join(bin, 'setsid'), `#!/bin/sh
exec /usr/bin/perl -MPOSIX -e 'POSIX::setsid(); exec @ARGV or die $!;' "$@"
`, { mode: 0o755 });
  }
  await fs.writeFile(path.join(deploy, 'compose.old.yml'), 'services: {}\n');
  await fs.writeFile(path.join(deploy, 'app.txt'), 'old-file');
  await fs.writeFile(path.join(volume, 'value.txt'), 'old-volume');
  await fs.writeFile(path.join(incoming, 'tree', 'deploy', 'app.txt'), 'new-file');
  await fs.writeFile(path.join(incoming, 'tree', 'deploy', 'new.txt'), 'new-entry');
  await fs.writeFile(path.join(incoming, 'tree', 'deploy', 'compose.new.yml'), 'services: {}\n');
  await fs.writeFile(path.join(txn, 'image-override.compose.json'), '{}\n');
  await fs.writeFile(path.join(txn, 'old-image-override.compose.json'), '{}\n');
  await fs.writeFile(
    path.join(incoming, 'managed-paths.txt'),
    'deploy/app.txt\ndeploy/new.txt\ndeploy/compose.old.yml\ndeploy/compose.new.yml\n'
  );
  await fs.writeFile(
    path.join(incoming, 'upload-paths.txt'),
    'deploy/app.txt\ndeploy/new.txt\ndeploy/compose.new.yml\n'
  );
  await fs.writeFile(path.join(incoming, 'remove-paths.txt'), '');
  await fs.writeFile(
    path.join(incoming, 'volume-specs.tsv'),
    'data\tmock-volume\tnginx:test\tyes\n'
  );
  await fs.writeFile(
    path.join(incoming, 'deployed.json'),
    JSON.stringify({ schemaVersion: 2, version: '2.0.0' })
  );
  return { root, bin, install, deploy, txn, volume };
}

async function runWorker(context, {
  mode = 'deploy',
  failNewUp = false,
  failNewDown = false,
  failOldDown = false,
  failRestore = false,
  hadDeployment = true,
  frontendPort = '',
  detached = false,
  newUpDelay = ''
} = {}) {
  const args = [
    mode,
    context.install,
    context.deploy,
    context.txn,
    hadDeployment ? 'yes' : 'no',
    hadDeployment ? 'compose.old.yml' : '',
    'compose.new.yml',
    'frontend',
    String(frontendPort),
    'frontend',
    'frontend',
    path.join(context.txn, 'image-override.compose.json'),
    path.join(context.txn, 'old-image-override.compose.json')
  ];
  if (detached) await fs.writeFile(path.join(context.txn, 'worker.sh'), versionDeployScript);
  const options = {
    env: {
      PATH: `${context.bin}:${process.env.PATH}`,
      MOCK_DOCKER_LOG: path.join(context.root, 'docker.log'),
      MOCK_VOLUME_DIR: context.volume,
      MOCK_FAIL_NEW_UP: failNewUp ? 'yes' : 'no',
      MOCK_FAIL_NEW_DOWN: failNewDown ? 'yes' : 'no',
      MOCK_FAIL_OLD_DOWN: failOldDown ? 'yes' : 'no',
      MOCK_NEW_UP_DELAY: newUpDelay,
      MOCK_FAIL_RESTORE: failRestore ? 'yes' : 'no'
    },
    timeoutMs: 30000
  };
  if (!detached) return runBashFile(worker, args, options);
  const launch = path.resolve(path.dirname(worker), 'version-deploy-launch.sh');
  return new Promise((resolve) => {
    execFile('/bin/bash', ['--noprofile', '--norc', '--', launch, context.txn, ...args], {
      env: { ...process.env, ...options.env },
      timeout: options.timeoutMs
    }, (error, stdout, stderr) => {
      resolve({
        code: error ? (Number.isInteger(error.code) ? error.code : 1) : 0,
        stdout,
        stderr
      });
    });
  });
}

function assertWorkerSucceeded(result) {
  assert.equal(
    result.code,
    0,
    `worker failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`
  );
}

test('远端 worker 成功提交完整版本并删除快照', async (t) => {
  const context = await fixture(t);
  const result = await runWorker(context);
  assertWorkerSucceeded(result);
  assert.equal(await fs.readFile(path.join(context.deploy, 'app.txt'), 'utf8'), 'new-file');
  assert.equal(await fs.readFile(path.join(context.deploy, 'new.txt'), 'utf8'), 'new-entry');
  assert.equal(await fs.readFile(path.join(context.volume, 'value.txt'), 'utf8'), 'old-volume');
  const status = await fs.readFile(path.join(context.txn, 'status.env'), 'utf8');
  assert.match(status, /phase=committed/);
  assert.match(status, /state=committed/);
  await assert.rejects(fs.stat(path.join(context.txn, 'snapshot')));
});

test('远端 worker 在新版本启动失败后恢复旧文件和旧数据卷', async (t) => {
  const context = await fixture(t);
  const result = await runWorker(context, { failNewUp: true });
  assertWorkerSucceeded(result);
  assert.equal(await fs.readFile(path.join(context.deploy, 'app.txt'), 'utf8'), 'old-file');
  await assert.rejects(fs.stat(path.join(context.deploy, 'new.txt')));
  await assert.rejects(fs.stat(path.join(context.deploy, 'compose.new.yml')));
  assert.equal(await fs.readFile(path.join(context.volume, 'value.txt'), 'utf8'), 'old-volume');
  const dockerLog = await fs.readFile(path.join(context.root, 'docker.log'), 'utf8');
  const oldUp = dockerLog.split('\n').find((line) =>
    line.includes('compose.old.yml') && line.includes(' up -d ')
  );
  assert.ok(oldUp?.includes(`${context.txn}/old-image-override.compose.json`));
  assert.ok(!oldUp?.includes(`${context.txn}/image-override.compose.json`));
  const status = await fs.readFile(path.join(context.txn, 'status.env'), 'utf8');
  assert.match(status, /phase=recovered/);
  assert.match(status, /state=recovered/);
});

test('远端 worker 首次部署失败后清理新文件和新数据卷', async (t) => {
  const context = await fixture(t);
  await fs.rm(path.join(context.deploy, 'app.txt'));
  await fs.rm(path.join(context.deploy, 'compose.old.yml'));
  await fs.writeFile(
    path.join(context.txn, 'incoming', 'managed-paths.txt'),
    'deploy/app.txt\ndeploy/new.txt\ndeploy/compose.new.yml\n'
  );
  await fs.writeFile(
    path.join(context.txn, 'incoming', 'volume-specs.tsv'),
    'data\tmock-volume\tnginx:test\tno\n'
  );
  const result = await runWorker(context, {
    failNewUp: true,
    hadDeployment: false
  });
  assert.notEqual(result.code, 0);
  await assert.rejects(fs.stat(path.join(context.deploy, 'app.txt')));
  await assert.rejects(fs.stat(path.join(context.deploy, 'new.txt')));
  await assert.rejects(fs.stat(path.join(context.deploy, 'compose.new.yml')));
  await assert.rejects(fs.stat(context.volume));
  const status = await fs.readFile(path.join(context.txn, 'status.env'), 'utf8');
  assert.match(status, /phase=recovered/);
  assert.match(status, /state=recovered/);
  assert.match(
    Buffer.from(status.match(/errorBase64=(.*)/)?.[1] || '', 'base64').toString('utf8'),
    /部署阶段 starting 失败/
  );
});

test('远端 worker 恢复数据卷失败时标记 recovery_failed 并保留现场', async (t) => {
  const context = await fixture(t);
  const result = await runWorker(context, {
    failNewUp: true,
    failRestore: true
  });
  assert.notEqual(result.code, 0);
  const status = await fs.readFile(path.join(context.txn, 'status.env'), 'utf8');
  assert.match(status, /phase=recovery_failed/);
  assert.match(status, /state=recovery_failed/);
  const recoveryError = Buffer.from(
    status.match(/recoveryErrorBase64=(.*)/)?.[1] || '',
    'base64'
  ).toString('utf8');
  assert.match(recoveryError, /自动恢复失败/);
  assert.equal((await fs.stat(path.join(context.txn, 'snapshot'))).isDirectory(), true);
});

test('远端 worker 停止失败版本失败时不写回旧文件', async (t) => {
  const context = await fixture(t);
  const result = await runWorker(context, {
    failNewUp: true,
    failNewDown: true
  });
  assert.notEqual(result.code, 0);
  assert.equal(await fs.readFile(path.join(context.deploy, 'app.txt'), 'utf8'), 'new-file');
  const status = await fs.readFile(path.join(context.txn, 'status.env'), 'utf8');
  assert.match(status, /phase=recovery_failed/);
  assert.match(status, /state=recovery_failed/);
});

test('远端 worker 从宿主机检查实际网页端口，无需容器中的 wget', async (t) => {
  const context = await fixture(t);
  const server = http.createServer((request, response) => response.end('ready'));
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));
  const result = await runWorker(context, { frontendPort: server.address().port });
  assertWorkerSucceeded(result);
  const dockerLog = await fs.readFile(path.join(context.root, 'docker.log'), 'utf8');
  assert.ok(!dockerLog.includes('wget'));
});

test('远端 worker 在预检基线变化时停服前退出', async (t) => {
  const context = await fixture(t);
  await fs.writeFile(path.join(context.txn, 'incoming', 'expected-baseline'), 'outdated');
  const result = await runWorker(context);
  assert.notEqual(result.code, 0);
  assert.equal(await fs.readFile(path.join(context.deploy, 'app.txt'), 'utf8'), 'old-file');
  const dockerLog = await fs.readFile(path.join(context.root, 'docker.log'), 'utf8');
  assert.ok(!dockerLog.includes(' down '));
  assert.match(result.stderr, /预检后发生变化/);
  await fs.writeFile(path.join(context.deploy, 'app.txt'), 'newer-version');
  await fs.writeFile(path.join(context.volume, 'value.txt'), 'newer-data');
  const before = await fs.readFile(path.join(context.root, 'docker.log'), 'utf8');
  assertWorkerSucceeded(await runWorker(context, { mode: 'recover' }));
  assert.equal(await fs.readFile(path.join(context.deploy, 'app.txt'), 'utf8'), 'newer-version');
  assert.equal(await fs.readFile(path.join(context.volume, 'value.txt'), 'utf8'), 'newer-data');
  const after = await fs.readFile(path.join(context.root, 'docker.log'), 'utf8');
  assert.equal(after.slice(before.length).trim(), 'compose version');
  const status = await fs.readFile(path.join(context.txn, 'status.env'), 'utf8');
  assert.match(status, /deploymentStarted=no/);
  assert.match(status, /state=recovered/);
});

test('旧版本停止失败时只恢复旧服务运行，不快照、不写文件或恢复数据卷', async (t) => {
  const context = await fixture(t);
  const result = await runWorker(context, { failOldDown: true });
  assertWorkerSucceeded(result);
  assert.equal(await fs.readFile(path.join(context.deploy, 'app.txt'), 'utf8'), 'old-file');
  assert.equal(await fs.readFile(path.join(context.volume, 'value.txt'), 'utf8'), 'old-volume');
  await assert.rejects(fs.stat(path.join(context.txn, 'snapshot')));
  const log = await fs.readFile(path.join(context.root, 'docker.log'), 'utf8');
  assert.ok(!log.includes('tar -czf'));
  assert.ok(!log.includes('tar -xzf'));
  assert.ok(!log.includes('compose.new.yml'));
  const status = await fs.readFile(path.join(context.txn, 'status.env'), 'utf8');
  assert.match(status, /state=recovered/);
  assert.match(result.stderr, /停止旧版本失败/);
});

test('detached worker 在启动连接退出后仍继续执行并可轮询最终状态', async (t) => {
  const context = await fixture(t);
  const result = await runWorker(context, { detached: true, newUpDelay: '0.8' });
  assertWorkerSucceeded(result);
  assert.match(result.stdout, /WORKER_STARTED/);
  let status = '';
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    status = await fs.readFile(path.join(context.txn, 'status.env'), 'utf8').catch(() => '');
    if (/state=(committed|recovered|recovery_failed|failed)/.test(status)) break;
    await new Promise((resolve) => setTimeout(resolve, 40));
  }
  if (!/state=committed/.test(status)) {
    const workerLog = await fs.readFile(path.join(context.txn, 'worker.log'), 'utf8').catch(() => '');
    const dockerLog = await fs.readFile(path.join(context.root, 'docker.log'), 'utf8').catch(() => '');
    assert.fail(`worker did not commit\nstatus:\n${status}\nworker log:\n${workerLog}\ndocker log:\n${dockerLog}`);
  }
  assert.match(status, /state=committed/);
  assert.equal(await fs.readFile(path.join(context.deploy, 'app.txt'), 'utf8'), 'new-file');
});

test('已完成恢复的事务再次恢复不会重写文件或数据卷', async (t) => {
  const context = await fixture(t);
  assertWorkerSucceeded(await runWorker(context, { failNewUp: true }));
  const before = await fs.readFile(path.join(context.root, 'docker.log'), 'utf8');
  const repeated = await runWorker(context, { mode: 'recover' });
  assert.notEqual(repeated.code, 0);
  const after = await fs.readFile(path.join(context.root, 'docker.log'), 'utf8');
  assert.equal(after.slice(before.length).trim(), 'compose version');
  assert.equal(await fs.readFile(path.join(context.deploy, 'app.txt'), 'utf8'), 'old-file');
});
