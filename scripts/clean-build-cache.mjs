#!/usr/bin/env node
// 清理 Rust/Tauri 的 debug 缓存。默认只删可再生的 incremental 产物，
// 保留依赖编译缓存（target/debug/deps）和 release 构建，避免下次构建从头开始。
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const hasFlag = (name) => args.includes(name);

const dryRun = hasFlag('--dry-run');
const deep = hasFlag('--deep');
const includeWorktrees = hasFlag('--worktrees');

if (hasFlag('--help') || hasFlag('-h')) {
  console.log(`用法：node scripts/clean-build-cache.mjs [选项]

默认清理：
  src-tauri/target/debug/incremental
  以及 target/<triple>/debug/incremental

选项：
  --deep       删除整个 target/debug（下次 debug 构建会重新编译依赖，耗时更长）
  --worktrees  同时清理本仓库已注册 worktree 中的同类缓存
  --dry-run    只列出将要删除的内容和大小
  --help       显示本帮助

始终保留：
  src-tauri/target/release、Cargo 注册表与 git 依赖缓存、node_modules。`);
  process.exit(0);
}

function formatBytes(bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function directorySize(directory) {
  let total = 0;
  const stack = [directory];
  const seenInodes = new Set();
  while (stack.length > 0) {
    const current = stack.pop();
    let entry;
    try {
      entry = fs.lstatSync(current);
    } catch {
      continue;
    }
    if (entry.isSymbolicLink()) continue;
    if (entry.isDirectory()) {
      let children = [];
      try {
        children = fs.readdirSync(current);
      } catch {
        continue;
      }
      for (const child of children) stack.push(path.join(current, child));
    } else if (entry.isFile()) {
      if (entry.nlink > 1) {
        const inode = `${entry.dev}:${entry.ino}`;
        if (seenInodes.has(inode)) continue;
        seenInodes.add(inode);
      }
      total += typeof entry.blocks === 'number' && entry.blocks > 0
        ? entry.blocks * 512
        : entry.size;
    }
  }
  return total;
}

function registeredWorktrees() {
  if (!includeWorktrees) return [];
  const result = spawnSync('git', ['worktree', 'list', '--porcelain'], {
    cwd: root,
    encoding: 'utf8',
  });
  if (result.status !== 0) {
    console.warn('无法读取 git worktree 列表，跳过 worktree 清理。');
    return [];
  }
  return result.stdout
    .split('\n')
    .filter((line) => line.startsWith('worktree '))
    .map((line) => path.resolve(line.slice('worktree '.length)))
    .filter((worktree) => worktree !== root);
}

function debugDirectories(projectRoot) {
  const targetDir = path.join(projectRoot, 'src-tauri', 'target');
  if (!fs.existsSync(targetDir)) return [];
  const directories = [path.join(targetDir, 'debug')];
  for (const entry of fs.readdirSync(targetDir, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name === 'debug' || entry.name === 'release') continue;
    directories.push(path.join(targetDir, entry.name, 'debug'));
  }
  return directories.filter((directory) => fs.existsSync(directory));
}

const projectRoots = [root, ...registeredWorktrees()];
const cleanupTargets = [];

for (const projectRoot of projectRoots) {
  for (const debugDir of debugDirectories(projectRoot)) {
    if (deep) {
      cleanupTargets.push(debugDir);
    } else {
      cleanupTargets.push(path.join(debugDir, 'incremental'));
    }
  }
}

const existingTargets = cleanupTargets.filter((target) => fs.existsSync(target));
if (existingTargets.length === 0) {
  console.log('没有找到可清理的 debug 缓存。');
  process.exit(0);
}

let totalBytes = 0;
const failures = [];

for (const target of existingTargets) {
  const bytes = directorySize(target);
  totalBytes += bytes;
  const displayPath = target.startsWith(`${root}${path.sep}`)
    ? path.relative(root, target)
    : target;
  console.log(`${dryRun ? '将删除' : '删除'} ${displayPath} (${formatBytes(bytes)})`);
  if (dryRun) continue;
  try {
    fs.rmSync(target, { recursive: true, force: true });
  } catch (error) {
    failures.push(`${displayPath}: ${error.message}`);
  }
}

console.log('');
console.log(`${dryRun ? '预计清理' : '已清理'} ${formatBytes(totalBytes)} 缓存。`);
console.log('已保留 target/debug/deps、target/release、Cargo 注册表和 node_modules。');
if (deep) {
  console.log('已使用 --deep：下次 debug 构建会重新编译依赖。');
}

if (failures.length > 0) {
  console.error('以下目录删除失败：');
  for (const failure of failures) console.error(`  ${failure}`);
  process.exit(1);
}
