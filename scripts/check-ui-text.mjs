import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const sourceRoot = path.resolve(scriptDir, '..', 'src');
const sourceExtensions = new Set(['.vue', '.js', '.css', '.html']);
const forbiddenPattern = /[\p{Extended_Pictographic}\u{2190}-\u{21FF}\u{2600}-\u{27BF}\u{2B00}-\u{2BFF}\u{FE0F}]/u;

async function collectFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];

  for (const entry of entries) {
    const fullPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...await collectFiles(fullPath));
    } else if (sourceExtensions.has(path.extname(entry.name))) {
      files.push(fullPath);
    }
  }

  return files;
}

const violations = [];

for (const file of await collectFiles(sourceRoot)) {
  const lines = (await readFile(file, 'utf8')).split('\n');
  lines.forEach((line, index) => {
    if (forbiddenPattern.test(line)) {
      violations.push(`${path.relative(sourceRoot, file)}:${index + 1}: ${line.trim()}`);
    }
  });
}

if (violations.length > 0) {
  console.error('UI 文本检查失败：deploy-desktop 中不允许使用 emoji 或装饰性符号。');
  violations.forEach((violation) => console.error(`  ${violation}`));
  process.exit(1);
}

console.log('UI 文本检查通过：未发现 emoji 或装饰性符号。');
