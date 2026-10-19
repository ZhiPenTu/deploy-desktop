import fs from 'node:fs/promises';
import path from 'node:path';
import { DeployError } from './errors.mjs';
import { requireBinary, run } from './exec.mjs';
import { dockerLoginLocal, dockerLoginRemote } from './registry.mjs';
import { explainSshFailure, runRemote, uploadFile } from './ssh.mjs';
import { expandHome, formatBytes, isArchiveFile, sanitizeFileName, shellQuote } from './util.mjs';

export async function resolveImagesDir(input) {
  const raw = path.resolve(expandHome(String(input || '').trim()));
  if (!raw) throw new DeployError('请填写镜像包目录');
  let stat;
  try {
    stat = await fs.stat(raw);
  } catch (error) {
    throw new DeployError(`目录不存在: ${raw}`, { code: 'DIR_NOT_FOUND', details: { error: error.message } });
  }
  if (!stat.isDirectory()) throw new DeployError(`路径不是目录: ${raw}`);
  if (path.basename(raw) === 'images') return raw;
  const nested = path.join(raw, 'images');
  const nestedStat = await fs.stat(nested).catch(() => null);
  if (nestedStat?.isDirectory()) return nested;
  return raw;
}

export function remoteImagesDir(bundleDir) {
  const root = String(bundleDir || '').trim().replace(/\/+$/, '');
  if (!root) throw new DeployError('请填写远程镜像包目录');
  return `${root}/images`;
}

export async function listArchiveFiles(dir) {
  const imagesDir = await resolveImagesDir(dir);
  const entries = await fs.readdir(imagesDir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    if (!isArchiveFile(entry.name)) continue;
    const full = path.join(imagesDir, entry.name);
    const stat = await fs.stat(full);
    files.push({ name: entry.name, sizeBytes: stat.size, modifiedMs: Math.floor(stat.mtimeMs) });
  }
  files.sort((a, b) => a.name.localeCompare(b.name));
  return { imagesDir, files };
}

function expectationsFor(profile, files) {
  return profile.images.map((image) => ({
    fileName: image.file,
    imageRef: image.imageRef,
    filePresent: files.some((file) => file.name === image.file)
  }));
}

export async function scanLocalBundle(profile, dir) {
  const { imagesDir, files } = await listArchiveFiles(dir);
  const expectations = expectationsFor(profile, files);
  const totalBytes = files.reduce((sum, file) => sum + file.sizeBytes, 0);
  const missing = expectations.filter((item) => !item.filePresent).map((item) => item.fileName);
  return {
    inputDir: path.resolve(expandHome(dir)),
    imagesDir,
    exists: true,
    files: files.map((file) => ({
      name: file.name,
      sizeBytes: file.sizeBytes,
      sizeText: formatBytes(file.sizeBytes),
      modifiedMs: file.modifiedMs,
      imageRef: profile.images.find((image) => image.file === file.name)?.imageRef || null
    })),
    totalBytes,
    totalText: formatBytes(totalBytes),
    expectations,
    missing,
    message: files.length
      ? `共 ${files.length} 个镜像包文件，合计 ${formatBytes(totalBytes)}`
      : '镜像目录中未找到 .tar/.tgz 文件'
  };
}

export async function dockerImageDigests(ref) {
  const docker = requireBinary('docker');
  const result = await run(docker, ['image', 'inspect', ref, '--format', '{{json .RepoDigests}}'], { timeoutMs: 30000 });
  if (result.code !== 0) return [];
  try {
    const parsed = JSON.parse(result.stdout.trim() || '[]');
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export async function dockerImageSize(ref) {
  const docker = requireBinary('docker');
  const result = await run(docker, ['image', 'inspect', ref, '--format', '{{.Size}}'], { timeoutMs: 30000 });
  return result.code === 0 ? Number(result.stdout.trim()) || 0 : 0;
}

export async function pullImageLocal(image, options = {}) {
  const docker = requireBinary('docker');
  const pullArgs = ['pull'];
  if (image.platform) pullArgs.push('--platform', image.platform);
  pullArgs.push(image.reference);
  const pull = await run(docker, pullArgs, {
    timeoutMs: options.timeoutMs ?? 30 * 60 * 1000,
    onStderr: options.onOutput
  });
  if (pull.code !== 0) {
    throw new DeployError(`拉取镜像 ${image.name || image.reference} 失败: ${pull.stderr.trim() || pull.stdout.trim()}`, {
      code: 'IMAGE_PULL_FAILED'
    });
  }
  if (image.digest) {
    const digests = await dockerImageDigests(image.reference);
    if (digests.length > 0 && !digests.some((digest) => digest.endsWith(`@${image.digest}`))) {
      throw new DeployError(`镜像 ${image.name || image.reference} 的 digest 校验失败，期望 ${image.digest}`, {
        code: 'IMAGE_DIGEST_MISMATCH'
      });
    }
  }
  let exportedPath = null;
  if (options.exportDir) {
    const dir = path.resolve(expandHome(options.exportDir));
    await fs.mkdir(dir, { recursive: true });
    const fileName = `${sanitizeFileName((image.name || image.reference.split('/').pop().split('@')[0]).replaceAll(':', '_'))}.tar`;
    const target = path.join(dir, fileName);
    const save = await run(docker, ['save', '-o', target, image.reference], {
      timeoutMs: options.timeoutMs ?? 30 * 60 * 1000
    });
    if (save.code !== 0) throw new DeployError(`导出镜像失败: ${save.stderr.trim()}`);
    exportedPath = target;
  }
  const sizeBytes = await dockerImageSize(image.reference);
  return {
    name: image.name || image.reference,
    reference: image.reference,
    digest: image.digest || null,
    platform: image.platform || null,
    exportedPath,
    sizeBytes,
    message: exportedPath ? '已拉取并导出' : '已拉取'
  };
}

export async function pullImageRemote(hostConfig, image, options = {}) {
  const pullArgs = image.platform ? `--platform ${shellQuote(image.platform)} ` : '';
  let exportScript = '';
  let exportPath = null;
  if (options.exportDir) {
    const dir = String(options.exportDir).replace(/\/+$/, '');
    const fileName = `${sanitizeFileName((image.name || image.reference.split('/').pop().split('@')[0]).replaceAll(':', '_'))}.tar`;
    exportPath = `${dir}/${fileName}`;
    exportScript = `mkdir -p ${shellQuote(dir)}
docker save -o ${shellQuote(exportPath)} ${shellQuote(image.reference)}
`;
  }
  const digestCheck = image.digest
    ? `if ! printf '%s\\n' "$DIGESTS" | grep -q ${shellQuote(`@${image.digest}`)}; then
  echo "DIGEST_MISMATCH" >&2
  exit 5
fi
`
    : '';
  const script = `set -e
docker pull ${pullArgs}${shellQuote(image.reference)} >/dev/null
DIGESTS=$(docker image inspect ${shellQuote(image.reference)} --format '{{range .RepoDigests}}{{println .}}{{end}}' 2>/dev/null || true)
${digestCheck}SIZE=$(docker image inspect ${shellQuote(image.reference)} --format '{{.Size}}' 2>/dev/null || echo 0)
${exportScript}if [ -n ${shellQuote(exportPath || '')} ]; then SIZE=$(stat -c '%s' ${shellQuote(exportPath)}); fi
echo "SIZE=$SIZE"
echo "DIGESTS=$DIGESTS"
`;
  const result = await runRemote(hostConfig, script, { timeoutMs: options.timeoutMs ?? 30 * 60 * 1000 });
  const sizeLine = result.stdout.split('\n').find((line) => line.startsWith('SIZE='));
  const sizeBytes = sizeLine ? Number(sizeLine.slice(5)) || 0 : 0;
  return {
    name: image.name || image.reference,
    reference: image.reference,
    digest: image.digest || null,
    platform: image.platform || null,
    exportedPath: exportPath,
    sizeBytes,
    message: exportPath ? '已拉取并导出' : '已拉取'
  };
}

export async function pullImages(profile, registryConfig, images, options = {}) {
  const target = options.target || 'local';
  if (target === 'remote' && !options.hostConfig) throw new DeployError('远程拉取需要 host 参数');
  if (registryConfig?.username) {
    if (target === 'remote') await dockerLoginRemote(options.hostConfig, registryConfig);
    else await dockerLoginLocal(registryConfig);
  }
  const results = [];
  for (const [index, image] of images.entries()) {
    options.onProgress?.(index, images.length, `正在拉取 ${image.name || image.reference}`);
    const result = target === 'remote'
      ? await pullImageRemote(options.hostConfig, image, options)
      : await pullImageLocal(image, options);
    results.push(result);
    options.onProgress?.(index + 1, images.length, `${image.name || image.reference} 完成`);
  }
  return results;
}

export async function loadLocalImages(bundleDir, files = []) {
  const { imagesDir, files: available } = await listArchiveFiles(bundleDir);
  const selected = files.length
    ? files.map((name) => {
        const clean = sanitizeFileName(name);
        if (!available.some((file) => file.name === clean)) throw new DeployError(`本地镜像包不存在: ${clean}`);
        return clean;
      })
    : available.map((file) => file.name);
  if (selected.length === 0) throw new DeployError(`镜像目录中未找到任何 .tar 镜像文件: ${imagesDir}`);
  const docker = requireBinary('docker');
  const loaded = [];
  for (const name of selected) {
    const filePath = path.join(imagesDir, name);
    const result = await run(docker, ['load', '-i', filePath], { timeoutMs: 30 * 60 * 1000 });
    if (result.code !== 0) throw new DeployError(`导入镜像 ${name} 失败: ${result.stderr.trim()}`);
    loaded.push({ file: name, output: result.stdout.trim() });
  }
  return loaded;
}

export async function loadRemoteImages(hostConfig, bundleDir, files = []) {
  const dir = remoteImagesDir(bundleDir);
  let selected = files.map((name) => sanitizeFileName(name));
  if (selected.length === 0) {
    const list = await runRemote(hostConfig, `set -e
IMAGES=${shellQuote(dir)}
[ -d "$IMAGES" ] || { echo "镜像目录不存在: $IMAGES" >&2; exit 3; }
find "$IMAGES" -maxdepth 1 -type f \\( -name '*.tar' -o -name '*.tgz' -o -name '*.tar.gz' \\) -printf '%f\\n' | sort
`);
    selected = list.stdout.split('\n').map((line) => line.trim()).filter(Boolean);
  }
  if (selected.length === 0) throw new DeployError(`远程镜像目录中未找到任何 .tar 镜像文件: ${dir}`);
  const loaded = [];
  for (const name of selected) {
    const remotePath = `${dir}/${name}`;
    const result = await runRemote(hostConfig, `set -e
[ -f ${shellQuote(remotePath)} ] || { echo "目标机镜像包文件不存在: ${remotePath}" >&2; exit 4; }
docker load -i ${shellQuote(remotePath)}
`, { timeoutMs: 30 * 60 * 1000 });
    loaded.push({ file: name, output: result.stdout.trim() });
  }
  return loaded;
}

function parseImageLines(stdout) {
  return stdout.split('\n').map((line) => line.trim()).filter(Boolean).map((line) => {
    try {
      return JSON.parse(line);
    } catch {
      return { raw: line };
    }
  });
}

export async function listLocalImages() {
  const docker = requireBinary('docker');
  const result = await run(docker, ['image', 'ls', '--format', '{{json .}}'], { timeoutMs: 30000 });
  if (result.code !== 0) throw new DeployError(`读取本地镜像失败: ${result.stderr.trim()}`);
  return parseImageLines(result.stdout);
}

export async function listRemoteImages(hostConfig) {
  const result = await runRemote(hostConfig, `docker image ls --format '{{json .}}'\n`, { timeoutMs: 60000 });
  return parseImageLines(result.stdout);
}

export async function removeImages(target, refs, options = {}) {
  if (!refs?.length) throw new DeployError('请指定要删除的镜像');
  if (options.confirm !== true) throw new DeployError('删除镜像需要显式确认：请传入 confirm=true', { code: 'CONFIRM_REQUIRED' });
  if (target === 'remote') {
    const script = `set -e
${refs.map((ref) => `docker rmi ${options.force ? '-f ' : ''}${shellQuote(ref)}`).join('\n')}
`;
    const result = await runRemote(options.hostConfig, script, { timeoutMs: 10 * 60 * 1000 });
    return { removed: refs, output: result.stdout.trim() };
  }
  const docker = requireBinary('docker');
  const args = ['rmi'];
  if (options.force) args.push('-f');
  args.push(...refs);
  const result = await run(docker, args, { timeoutMs: 10 * 60 * 1000 });
  if (result.code !== 0) throw new DeployError(`删除镜像失败: ${result.stderr.trim()}`);
  return { removed: refs, output: result.stdout.trim() };
}

export async function saveImages(target, refs, dir, options = {}) {
  if (!refs?.length) throw new DeployError('请指定要导出的镜像');
  if (!dir) throw new DeployError('请指定导出目录');
  const results = [];
  for (const ref of refs) {
    const name = sanitizeFileName(ref.split('/').pop().split('@')[0].replace(/:/g, '_'));
    const fileName = `${name}.tar`;
    if (target === 'remote') {
      const remotePath = `${String(dir).replace(/\/+$/, '')}/${fileName}`;
      const result = await runRemote(options.hostConfig, `set -e
mkdir -p ${shellQuote(dir)}
docker save -o ${shellQuote(remotePath)} ${shellQuote(ref)}
stat -c '%s' ${shellQuote(remotePath)}
`, { timeoutMs: 30 * 60 * 1000 });
      results.push({ reference: ref, path: remotePath, sizeBytes: Number(result.stdout.trim().split('\n').pop()) || 0 });
    } else {
      const localDir = path.resolve(expandHome(dir));
      await fs.mkdir(localDir, { recursive: true });
      const targetPath = path.join(localDir, fileName);
      const docker = requireBinary('docker');
      const result = await run(docker, ['save', '-o', targetPath, ref], { timeoutMs: 30 * 60 * 1000 });
      if (result.code !== 0) throw new DeployError(`导出镜像 ${ref} 失败: ${result.stderr.trim()}`);
      const stat = await fs.stat(targetPath);
      results.push({ reference: ref, path: targetPath, sizeBytes: stat.size });
    }
  }
  return results;
}

export async function inspectRemoteBundle(profile, hostConfig, bundleDir) {
  const dir = remoteImagesDir(bundleDir);
  const result = await runRemote(hostConfig, `set -e
IMAGES=${shellQuote(dir)}
if [ -d "$IMAGES" ]; then echo "EXISTS=yes"; else echo "EXISTS=no"; fi
DF_PATH="$IMAGES"
while [ ! -d "$DF_PATH" ] && [ "$DF_PATH" != "/" ]; do DF_PATH=$(dirname "$DF_PATH"); done
echo "FREE_KB=$(df -Pk "$DF_PATH" 2>/dev/null | awk 'NR==2{print $4}')"
if command -v docker >/dev/null 2>&1; then echo "DOCKER_AVAILABLE=yes"; else echo "DOCKER_AVAILABLE=no"; fi
if docker info >/dev/null 2>&1; then echo "DOCKER_RUNNING=yes"; else echo "DOCKER_RUNNING=no"; fi
echo "---FILES---"
if [ -d "$IMAGES" ]; then
  find "$IMAGES" -maxdepth 1 -type f \\( -name '*.tar' -o -name '*.tgz' -o -name '*.tar.gz' \\) -print | sort | while IFS= read -r f; do
    printf 'FILE|%s|%s|%s\\n' "$(basename "$f")" "$(stat -c '%s' "$f")" "$(stat -c '%Y' "$f")"
  done
fi
echo "---IMAGES---"
if docker info >/dev/null 2>&1; then
  docker image ls --format '{{.Repository}}:{{.Tag}}' 2>/dev/null | sort | sed 's/^/IMAGE|/'
fi
`, { timeoutMs: 60000 });
  const files = [];
  const dockerImages = [];
  let exists = false;
  let freeKb = 0;
  let dockerAvailable = false;
  let dockerRunning = false;
  let section = '';
  for (const line of result.stdout.split('\n')) {
    if (line === '---FILES---') { section = 'files'; continue; }
    if (line === '---IMAGES---') { section = 'images'; continue; }
    if (line.startsWith('EXISTS=')) exists = line.slice(7) === 'yes';
    else if (line.startsWith('FREE_KB=')) freeKb = Number(line.slice(8)) || 0;
    else if (line.startsWith('DOCKER_AVAILABLE=')) dockerAvailable = line.slice(17) === 'yes';
    else if (line.startsWith('DOCKER_RUNNING=')) dockerRunning = line.slice(15) === 'yes';
    else if (section === 'files' && line.startsWith('FILE|')) {
      const [, name, size, mtime] = line.split('|');
      files.push({ name, sizeBytes: Number(size) || 0, modifiedMs: Math.floor((Number(mtime) || 0) * 1000) });
    } else if (section === 'images' && line.startsWith('IMAGE|')) {
      const image = line.slice(6).trim();
      if (image && !image.includes('<none>')) dockerImages.push(image);
    }
  }
  const expectations = expectationsFor(profile, files);
  const missingImages = profile.images
    .map((image) => image.imageRef)
    .filter((ref) => !dockerImages.includes(ref));
  const issues = [];
  if (!exists) issues.push(`目标机镜像目录不存在: ${dir}`);
  if (!dockerAvailable) issues.push('目标机未安装 Docker');
  else if (!dockerRunning) issues.push('Docker 守护进程未运行');
  if (exists && files.length === 0) issues.push('目标机镜像目录中没有 .tar 镜像文件');
  if (dockerRunning && missingImages.length > 0) issues.push(`${missingImages.length} 个镜像尚未导入 Docker`);
  return {
    bundleDir,
    imagesDir: dir,
    exists,
    files: files.map((file) => ({
      ...file,
      imageRef: profile.images.find((image) => image.file === file.name)?.imageRef || null
    })),
    totalBytes: files.reduce((sum, file) => sum + file.sizeBytes, 0),
    dockerAvailable,
    dockerRunning,
    dockerImages,
    expectations,
    missingImages,
    freeKb,
    issues
  };
}

export async function remoteImageSizes(hostConfig, remoteDir) {
  const result = await runRemote(hostConfig, `set -e
DIR=${shellQuote(remoteDir)}
if [ -d "$DIR" ]; then
  find "$DIR" -maxdepth 1 -type f -print | sort | while IFS= read -r f; do
    printf 'FILE|%s|%s\\n' "$(basename "$f")" "$(stat -c '%s' "$f")"
  done
fi
`, { timeoutMs: 60000 });
  const sizes = new Map();
  for (const line of result.stdout.split('\n')) {
    if (!line.startsWith('FILE|')) continue;
    const [, name, size] = line.split('|');
    sizes.set(name, Number(size) || 0);
  }
  return sizes;
}

export async function uploadBundle(hostConfig, localDir, remoteBundleDir, files = [], options = {}) {
  const { imagesDir, files: available } = await listArchiveFiles(localDir);
  const selected = files.length ? files.map((name) => sanitizeFileName(name)) : available.map((file) => file.name);
  if (selected.length === 0) throw new DeployError('没有可上传的镜像包文件');
  const remoteDir = remoteImagesDir(remoteBundleDir);
  await runRemote(hostConfig, `mkdir -p ${shellQuote(remoteDir)}\n`, { timeoutMs: 30000 });
  const remoteSizes = await remoteImageSizes(hostConfig, remoteDir);
  const messages = [];
  for (const name of selected) {
    const localPath = path.join(imagesDir, name);
    const stat = await fs.stat(localPath).catch(() => null);
    if (!stat) throw new DeployError(`本地镜像包文件不存在: ${localPath}`);
    if (!options.overwrite && remoteSizes.get(name) === stat.size) {
      messages.push(`${name} 已存在且大小一致，跳过上传`);
      continue;
    }
    await uploadFile(hostConfig, localPath, `${remoteDir}/${name}`, { timeoutMs: 60 * 60 * 1000 });
    messages.push(`${name} 上传完成（${formatBytes(stat.size)}）`);
  }
  return messages;
}

export async function removeRemoteBundleFiles(hostConfig, remoteBundleDir, files) {
  if (!files?.length) throw new DeployError('请选择要删除的镜像包文件');
  const dir = remoteImagesDir(remoteBundleDir);
  const script = `set -e
${files.map((name) => `rm -f -- ${shellQuote(`${dir}/${sanitizeFileName(name)}`)}`).join('\n')}
`;
  await runRemote(hostConfig, script, { timeoutMs: 60000 });
  return { removed: files };
}
