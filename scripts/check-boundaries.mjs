import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse, compileScript, compileTemplate } from 'vue/compiler-sfc';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const files = new Map();
async function collect(directory) {
  for (const entry of await readdir(path.join(root, directory), { withFileTypes: true })) {
    const file = `${directory}/${entry.name}`;
    if (entry.isDirectory()) await collect(file);
    else if (/\.(js|vue)$/.test(file)) files.set(file, await readFile(path.join(root, file), 'utf8'));
  }
}
await collect('src');

const problems = [];
const graph = new Map();
for (const [file, source] of files) {
  if (file.endsWith('.vue')) {
    const { descriptor } = parse(source);
    const script = compileScript(descriptor, { id: file });
    const template = compileTemplate({
      source: descriptor.template.content, filename: file, id: file,
      compilerOptions: { bindingMetadata: script.bindings }
    });
    for (const error of template.errors) problems.push(`${file}: ${String(error)}`);
    const missing = new Set([...template.code.matchAll(/_ctx\.([\w$]+)/g)].map((match) => match[1]));
    for (const name of missing) {
      if (!name.startsWith('$')) problems.push(`${file}: 模板绑定未定义 ${name}`);
    }
  }
  const imports = [...source.matchAll(/\b(?:from\s*|import\s*(?:\(\s*)?)['"]([^'"]+)['"]/g)].map((match) => match[1]);
  const dependencies = [];
  for (const specifier of imports) {
    if (specifier.startsWith('@tauri-apps/') && file !== 'src/api/desktop.js') {
      problems.push(`${file}: 桌面运行时只能由 src/api/desktop.js 引入`);
    }
    if (file.includes('/domain/') && !specifier.startsWith('.')) {
      problems.push(`${file}: 领域规则不能依赖运行时或界面库`);
    }
    if (!specifier.startsWith('.')) continue;
    let target = path.posix.normalize(path.posix.join(path.posix.dirname(file), specifier));
    if (files.has(`${target}.js`)) target += '.js';
    if (target.startsWith('src/api/') && !file.startsWith('src/api/') && target !== 'src/api/tauri.js') {
      problems.push(`${file}: 功能模块只能通过 src/api/tauri.js 使用桌面 API`);
    }
    if (file.startsWith('src/api/') && (target.startsWith('src/features/') || target.startsWith('src/components/'))) {
      problems.push(`${file}: API 层不能依赖功能界面`);
    }
    if (file.startsWith('src/shared/') && !target.startsWith('src/shared/')) {
      problems.push(`${file}: 通用异步工具不能依赖具体功能`);
    }
    if (file.includes('/domain/') && !target.includes('/domain/')) {
      problems.push(`${file}: 领域规则不能依赖副作用代码`);
    }
    if (files.has(target)) dependencies.push(target);
    else if (/\.(js|vue)$/.test(target)) problems.push(`${file}: 引用文件不存在 ${target}`);
  }
  graph.set(file, dependencies);
}

const visited = new Set();
const visiting = new Set();
function visit(file, chain) {
  if (visiting.has(file)) { problems.push(`循环依赖: ${[...chain, file].join(' -> ')}`); return; }
  if (visited.has(file)) return;
  visiting.add(file);
  for (const target of graph.get(file) || []) visit(target, [...chain, file]);
  visiting.delete(file);
  visited.add(file);
}
for (const file of files.keys()) visit(file, []);
if (problems.length) {
  console.error(problems.join('\n'));
  process.exitCode = 1;
} else {
  console.log(`模块边界检查通过：${files.size} 个文件，无缺失引用、越界引用、循环依赖或未定义模板绑定。`);
}
