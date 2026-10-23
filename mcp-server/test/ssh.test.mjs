import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { buildSshArgs } from '../src/ssh.mjs';

test('buildSshArgs 把远端命令放在目标主机之后', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'deploy-mcp-ssh-'));
  const key = path.join(dir, 'id_test');
  const previous = process.env.DEPLOY_DESKTOP_SSH_KEY;
  try {
    await fs.writeFile(key, 'test-key');
    await fs.chmod(key, 0o600);
    process.env.DEPLOY_DESKTOP_SSH_KEY = key;
    const args = buildSshArgs({ host: '192.168.1.10', port: 2222, user: 'deploy' }, ['bash -s']);
    const destination = args.indexOf('deploy@192.168.1.10');
    assert.ok(destination > 0, '命令中应包含目标主机');
    assert.equal(args[destination + 1], 'bash -s');
    assert.equal(args[args.length - 1], 'bash -s');
    assert.equal(args[destination - 2], '-i');
    assert.equal(args.indexOf('2222'), args.indexOf('-p') + 1);
  } finally {
    if (previous === undefined) delete process.env.DEPLOY_DESKTOP_SSH_KEY;
    else process.env.DEPLOY_DESKTOP_SSH_KEY = previous;
    await fs.rm(dir, { recursive: true, force: true });
  }
});

// Exercise the real SSH adapter against a local transport shim, never a remote host.
test('分阶段上传检查两端结果，生产者失败保留旧目录', { skip: process.platform === 'win32' }, async (t) => {
  const { uploadFile, uploadDir } = await import('../src/ssh.mjs');
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'deploy-transport-'));
  const bin = path.join(dir, 'bin');
  await fs.mkdir(bin);
  const previousPath = process.env.PATH;
  const previousKey = process.env.DEPLOY_DESKTOP_SSH_KEY;
  const key = path.join(dir, 'key');
  await fs.writeFile(key, 'fixture', { mode: 0o600 });
  await fs.writeFile(path.join(bin, 'ssh'), '#!/bin/bash\nexec /bin/bash -c "${!#}"\n', { mode: 0o755 });
  const realTar = (await import('../src/exec.mjs')).requireBinary('tar');
  process.env.PATH = `${bin}:${previousPath}`;
  process.env.DEPLOY_DESKTOP_SSH_KEY = key;
  t.after(async () => {
    process.env.PATH = previousPath;
    if (previousKey === undefined) delete process.env.DEPLOY_DESKTOP_SSH_KEY;
    else process.env.DEPLOY_DESKTOP_SSH_KEY = previousKey;
    await fs.rm(dir, { recursive: true, force: true });
  });
  const host = { host: 'transport.invalid', user: 'fixture' };
  const source = path.join(dir, 'source');
  await fs.mkdir(source);
  const payload = Buffer.alloc(200000, 0xa5);
  await fs.writeFile(path.join(source, 'file'), payload);
  const target = path.join(dir, "nested space/target ' $(literal)");
  await uploadFile(host, path.join(source, 'file'), `${target}.bin`);
  assert.deepEqual(await fs.readFile(`${target}.bin`), payload);
  await uploadDir(host, source, target);
  assert.deepEqual(await fs.readFile(path.join(target, 'file')), payload);
  await fs.writeFile(path.join(source, 'file'), 'new contents');
  // The receiver sees a valid archive, but the producer exits nonzero afterwards.
  await fs.writeFile(path.join(bin, 'tar'), `#!/bin/bash\n'${realTar}' "$@"\ncase "$1" in -czf) echo 'packing failed' >&2; exit 7;; esac\n`, { mode: 0o755 });
  await assert.rejects(uploadDir(host, source, target), (error) => error.code === 'PRODUCER_FAILED');
  assert.deepEqual(await fs.readFile(path.join(target, 'file')), payload);
  assert.deepEqual((await fs.readdir(path.dirname(target))).sort(), [path.basename(target), `${path.basename(target)}.bin`].sort());
});
