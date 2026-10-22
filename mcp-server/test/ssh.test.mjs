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
