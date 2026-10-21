import assert from 'node:assert/strict';
import { test } from 'node:test';
import { buildConfigDiff, serviceDiffRow } from '../src/features/deployment/services/configDiff.js';

test('same-image configuration changes align old and new values without omitting the rest of the file', () => {
  const current = '# deployment\nservices:\n  kafka:\n    image: example/kafka:4.0\n    ports: ["9092:9092"]\n  redis:\n    image: redis:7\nvolumes:\n  data: {}\n';
  const desired = current.replace('9092:9092', '19092:9092').replace('  data: {}', '  data: {}\n  cache: {}');
  const diff = buildConfigDiff(current, desired);
  assert.equal(diff.changes.length, 2);
  const port = diff.rows.find((row) => row.oldText.includes('ports:'));
  assert.equal(port.type, 'modified');
  assert.equal(port.oldLine, 5);
  assert.equal(port.newLine, 5);
  assert.match(port.newText, /19092/);
  assert.equal(diff.rows.at(-1).type, 'added');
  assert.equal(diff.rows.at(-1).oldLine, null);
  assert.equal(diff.rows.filter((row) => row.oldLine).map((row) => row.oldText).join('\n') + '\n', current);
  assert.equal(diff.rows.filter((row) => row.newLine).map((row) => row.newText).join('\n') + '\n', desired);
  assert.equal(serviceDiffRow(diff.rows, 'kafka'), 4);
  assert.equal(serviceDiffRow(diff.rows, 'redis'), 5);
});

test('insertions and deletions keep subsequent matching lines aligned and retain blank lines', () => {
  const diff = buildConfigDiff('a\nold\nremove\n\nb\nc\n', 'a\nnew\n\nb\ninsert\nc\n');
  assert.deepEqual(diff.rows.filter((row) => !row.changed).map((row) => row.oldText), ['a', '', 'b', 'c']);
  const c = diff.rows.at(-1);
  assert.equal(c.oldLine, 6);
  assert.equal(c.newLine, 6);
  assert.equal(diff.rows.filter((row) => row.type === 'removed').length, 1);
  assert.equal(diff.rows.filter((row) => row.type === 'added').length, 1);
});

test('empty, first-deployment and removed-file snapshots have accurate line counts', () => {
  assert.deepEqual(buildConfigDiff('', '').rows, []);
  assert.ok(buildConfigDiff('', 'services:\n  web: {}\n').rows.every((row) => row.oldLine === null && row.type === 'added'));
  assert.ok(buildConfigDiff('services:\n  web: {}\n', '').rows.every((row) => row.newLine === null && row.type === 'removed'));
  assert.equal(buildConfigDiff('a\r\n\r\nb\r\n', 'a\n\nb\n').changes.length, 0);
  const eof = buildConfigDiff('a', 'a\n');
  assert.equal(eof.changes.length, 1);
  assert.equal(eof.oldNoNewline, true);
  assert.equal(eof.newNoNewline, false);
});

test('secret-only changes stay highlighted while scalar, block and private-key values remain hidden', () => {
  const current = 'services:\n  kafka:\n    environment:\n      - KAFKA_PASSWORD=old-private-value\n      API_KEY: private-api-value\n      token: |\n        private-block-value\n      PRIVATE_KEY: |\n        -----BEGIN PRIVATE KEY-----\n        private-key-material\n        -----END PRIVATE KEY-----\n    image: example/kafka:4.0\n';
  const desired = current.replace('old-private-value', 'new-private-value');
  const diff = buildConfigDiff(current, desired);
  const display = JSON.stringify(diff);
  for (const secret of ['old-private-value', 'new-private-value', 'private-api-value', 'private-block-value', 'private-key-material']) assert.equal(display.includes(secret), false, secret);
  assert.equal(diff.changes.length, 1);
  assert.equal(diff.rows[3].changed, true);
  assert.equal(diff.rows[3].oldText, diff.rows[3].newText);
  assert.equal(diff.rows.at(-1).newLine, 12);
  assert.equal(diff.redacted, true);
});

test('complete configurations are retained beyond the previous inspector line limit', () => {
  const current = Array.from({ length: 3200 }, (_, index) => `# config line ${index}`).join('\n') + '\n';
  const desired = current.replace('# config line 3199', '# final change');
  const diff = buildConfigDiff(current, desired);
  assert.equal(diff.rows.length, 3200);
  assert.equal(diff.rows.at(-1).newText, '# final change');
  assert.deepEqual(diff.changes, [3199]);
});
