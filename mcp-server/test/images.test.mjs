import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import {
  listArchiveFiles,
  remoteImagesDir,
  resolveImagesDir,
  scanLocalBundle
} from '../src/images.mjs';
import { makeProfile, makeTempDir } from './helpers.mjs';

async function createBundle() {
  const root = await makeTempDir();
  const images = path.join(root, 'images');
  await fs.mkdir(images, { recursive: true });
  await fs.writeFile(path.join(images, 'b.tgz'), Buffer.alloc(2048));
  await fs.writeFile(path.join(images, 'a.tar'), Buffer.alloc(1024));
  await fs.writeFile(path.join(images, 'notes.txt'), 'ignore me');
  await fs.mkdir(path.join(images, 'nested'));
  return { root, images };
}

test('resolveImagesDir 自动识别 images 子目录', async () => {
  const { root, images } = await createBundle();
  assert.equal(await resolveImagesDir(root), images);
  assert.equal(await resolveImagesDir(images), images);
  await assert.rejects(resolveImagesDir(path.join(root, 'missing')), (error) => error.code === 'DIR_NOT_FOUND');
});

test('listArchiveFiles 只返回镜像包并排序', async () => {
  const { root } = await createBundle();
  const result = await listArchiveFiles(root);
  assert.deepEqual(result.files.map((file) => file.name), ['a.tar', 'b.tgz']);
  assert.equal(result.files[0].sizeBytes, 1024);
  assert.equal(Number.isInteger(result.files[0].modifiedMs), true);
});

test('scanLocalBundle 汇总大小并计算缺失文件', async () => {
  const { root } = await createBundle();
  const profile = makeProfile({
    images: [
      { file: 'a.tar', imageRef: 'demo/a:1' },
      { file: 'missing.tar', imageRef: 'demo/missing:1' }
    ]
  });
  const result = await scanLocalBundle(profile, root);
  assert.equal(result.exists, true);
  assert.equal(result.files.length, 2);
  assert.equal(result.totalBytes, 3072);
  assert.deepEqual(result.missing, ['missing.tar']);
  assert.equal(result.expectations.find((item) => item.fileName === 'a.tar').filePresent, true);
  assert.match(result.message, /共 2 个镜像包文件/);
});

test('scanLocalBundle 对空目录给出提示', async () => {
  const root = await makeTempDir();
  const images = path.join(root, 'images');
  await fs.mkdir(images, { recursive: true });
  const result = await scanLocalBundle(makeProfile(), root);
  assert.equal(result.files.length, 0);
  assert.equal(result.totalBytes, 0);
  assert.match(result.message, /未找到 \.tar\/\.tgz 文件/);
});

test('remoteImagesDir 规范化末尾斜杠', () => {
  assert.equal(remoteImagesDir('/opt/deploy/bundle/'), '/opt/deploy/bundle/images');
  assert.throws(() => remoteImagesDir(''), /远程镜像包目录/);
});
