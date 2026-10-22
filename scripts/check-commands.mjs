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
const rustFiles = await collect('src-tauri/src', '.rs');
for (const file of rustFiles) {
  const source = await read(file);
  for (const match of source.matchAll(/\.emit(?:_to)?\(\s*['"]([\w-]+)['"]/g)) {
    emittedEvents.set(match[1], file);
  }
}

const BLOCKING_CALLS = ['Command::new(', 'reqwest::blocking', 'run_ssh_script(', 'TcpStream::connect('];

/** 跳过字符串字面量，避免其中的花括号干扰函数体边界识别。 */
function skipString(source, open) {
  for (let index = open + 1; index < source.length; index += 1) {
    if (source[index] === '\\') {
      index += 1;
    } else if (source[index] === '"') {
      return index;
    }
  }
  return source.length;
}

function matchBrace(source, open) {
  let depth = 0;
  for (let index = open; index < source.length; index += 1) {
    const character = source[index];
    if (character === '"') {
      index = skipString(source, index);
    } else if (character === '/' && source[index + 1] === '/') {
      index = source.indexOf('\n', index);
      if (index < 0) return -1;
    } else if (character === '{') {
      depth += 1;
    } else if (character === '}') {
      depth -= 1;
      if (depth === 0) return index;
    }
  }
  return -1;
}

/** 收集 `#[tauri::command]` 标注的函数名、是否异步以及函数体文本。 */
function collectCommands(source) {
  const attribute = '#[tauri::command]';
  const commands = [];
  let cursor = 0;
  for (;;) {
    const at = source.indexOf(attribute, cursor);
    if (at < 0) return commands;
    cursor = at + attribute.length;
    const head = /^[\s\S]{0,400}?\bpub\s+(async\s+)?fn\s+([a-z0-9_]+)/.exec(source.slice(cursor));
    if (!head) continue;
    const open = source.indexOf('{', cursor + head[0].length);
    if (open < 0) continue;
    const close = matchBrace(source, open);
    if (close < 0) continue;
    commands.push({
      name: head[2],
      isAsync: Boolean(head[1]),
      body: source.slice(open, close + 1),
    });
  }
}

const problems = [];
for (const [command, file] of invoked) {
  if (!registered.has(command)) problems.push(`${file} 调用了未在 src-tauri/src/lib.rs 注册的命令 ${command}`);
}
for (const [event, file] of listenedEvents) {
  if (!emittedEvents.has(event)) problems.push(`${file} 监听了 Rust 侧从未发射的进度事件 ${event}`);
}

let checkedCommands = 0;
let syncCommands = 0;
for (const file of rustFiles) {
  for (const command of collectCommands(await read(file))) {
    checkedCommands += 1;
    if (command.isAsync) continue;
    syncCommands += 1;
    for (const call of BLOCKING_CALLS) {
      if (!command.body.includes(call)) continue;
      problems.push(
        `${file} 的同步命令 ${command.name} 直接调用 ${call.slice(0, -1)}，请改为 blocking::run 包装`
      );
    }
  }
}

if (problems.length > 0) {
  console.error('命令与事件检查失败：');
  problems.forEach((problem) => console.error(`  ${problem}`));
  process.exit(1);
}

console.log(
  `命令与事件检查通过：${invoked.size} 个命令均已在 Rust 注册，${listenedEvents.size} 个进度事件均有发射点；${checkedCommands} 个命令中 ${syncCommands} 个同步命令均未直接执行外部进程或网络请求。`
);
