import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

async function read(relative) {
  return await readFile(path.join(root, relative), 'utf8');
}

async function collect(directory, extension) {
  const files = [];
  for (const entry of await readdir(path.join(root, directory), { withFileTypes: true })) {
    const relative = `${directory}/${entry.name}`;
    if (entry.isDirectory()) files.push(...await collect(relative, extension));
    else if (relative.endsWith(extension)) files.push(relative);
  }
  return files;
}

const apiFiles = ['src/api/desktop.js'];
for (const entry of await readdir(path.join(root, 'src/api/commands'))) {
  if (entry.endsWith('.js')) apiFiles.push(`src/api/commands/${entry}`);
}

const invoked = new Map();
const listenedEvents = new Map();
for (const file of apiFiles) {
  const source = await read(file);
  for (const match of source.matchAll(/\b(?:invoke|invokeDesktop|invokeRead|invokeWithProgress)\(\s*['"]([a-z0-9_]+)['"]/g)) {
    invoked.set(match[1], file);
  }
  for (const match of source.matchAll(/\blistenProgress\(\s*['"]([\w-]+)['"]/g)) {
    listenedEvents.set(match[1], file);
  }
}

const lib = await read('src-tauri/src/lib.rs');
const start = lib.indexOf('generate_handler![');
const end = lib.lastIndexOf('])');
if (start < 0 || end < start) {
  console.error('命令检查失败：src-tauri/src/lib.rs 中未找到 generate_handler! 注册列表。');
  process.exit(1);
}
const registered = new Set(
  [...lib.slice(start, end).matchAll(/^\s*([a-z0-9_]+),\s*$/gm)].map((match) => match[1])
);

const emittedEvents = new Map();
for (const file of await collect('src-tauri/src', '.rs')) {
  const source = await read(file);
  for (const match of source.matchAll(/\.emit(?:_to)?\(\s*['"]([\w-]+)['"]/g)) {
    emittedEvents.set(match[1], file);
  }
}

const problems = [];
for (const [command, file] of invoked) {
  if (!registered.has(command)) problems.push(`${file} 调用了未在 src-tauri/src/lib.rs 注册的命令 ${command}`);
}
for (const [event, file] of listenedEvents) {
  if (!emittedEvents.has(event)) problems.push(`${file} 监听了 Rust 侧从未发射的进度事件 ${event}`);
}

if (problems.length > 0) {
  console.error('命令与事件检查失败：');
  problems.forEach((problem) => console.error(`  ${problem}`));
  process.exit(1);
}

console.log(
  `命令与事件检查通过：${invoked.size} 个命令均已在 Rust 注册，${listenedEvents.size} 个进度事件均有发射点。`
);
