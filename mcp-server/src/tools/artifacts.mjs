import { z } from 'zod';
import { DeployError } from '../errors.mjs';
import { formatBytes } from '../util.mjs';
import {
  artifactCacheRoot,
  artifactSourcePath,
  loadArtifactSources,
  mutateArtifactSources,
  nextArtifactSourceId,
  readArtifactToken,
  removeArtifactToken,
  removeReleaseCacheEntry,
  requireArtifactSource,
  validateArtifactSource,
  writeArtifactToken
} from '../artifact-sources.mjs';
import { downloadArtifact, listArtifactReleases, resolveArtifactRelease } from '../artifacts.mjs';
import { readOnly, remoteReadOnly, mutating, destructive } from './shared.mjs';
import { progressReporter } from './runtime.mjs';

const sourceSelector = z.string().min(1).describe('制品源 ID 或名称');

async function resolveSource(selector) {
  const sources = await loadArtifactSources();
  return requireArtifactSource(sources, selector);
}

function mergeSourceInput(args, existing) {
  const input = existing ? { ...existing } : {};
  for (const key of ['name', 'type', 'orgId', 'pipelineId', 'artifactName', 'version', 'downloadUrl']) {
    if (args[key] !== undefined) input[key] = args[key];
  }
  return input;
}

function publicSource(source) {
  return {
    id: source.id,
    name: source.name,
    type: source.type,
    orgId: source.orgId,
    pipelineId: source.pipelineId,
    artifactName: source.artifactName,
    version: source.version,
    downloadUrl: source.downloadUrl,
    hasToken: source.hasToken,
    lastVersion: source.lastVersion,
    lastBundleDir: source.lastBundleDir
  };
}

export function registerArtifactsTools(register) {
  register('deploy_artifact_source_list', {
    title: '查看制品源',
    description: '列出云效流水线与直接下载类型的制品源；不会返回令牌内容。',
    inputSchema: {},
    annotations: readOnly
  }, async () => ({
    path: artifactSourcePath(),
    bundleRoot: artifactCacheRoot(),
    sources: (await loadArtifactSources()).map(publicSource)
  }));

  register('deploy_artifact_source_save', {
    title: '保存制品源',
    description: '新增或更新制品源。云效源需要组织与流水线 ID；直接下载源需要版本与下载地址。令牌只保存到本机受保护配置。',
    inputSchema: {
      id: z.string().optional().describe('已有制品源 ID；留空表示新增'),
      name: z.string().min(1).describe('制品源名称，最多 60 个字符'),
      type: z.enum(['yunxiaoFlow', 'url']).optional().describe('yunxiaoFlow=云效流水线，url=直接下载；更新时留空保持不变'),
      orgId: z.string().optional().describe('云效组织 ID'),
      pipelineId: z.string().optional().describe('云效流水线 ID'),
      artifactName: z.string().optional().describe('流水线制品名，默认 deploy-bundle'),
      version: z.string().optional().describe('直接下载源的版本号'),
      downloadUrl: z.string().optional().describe('直接下载源的 HTTPS 地址'),
      token: z.string().optional().describe('云效个人访问令牌；仅用于本次保存，不会出现在返回结果中'),
      clearToken: z.boolean().optional().default(false).describe('清除已保存的令牌')
    },
    annotations: mutating
  }, async (args) => {
    const sources = await loadArtifactSources();
    const requestedId = String(args.id ?? '').trim();
    const existing = requestedId ? sources.find((source) => source.id === requestedId) || null : null;
    if (requestedId && !existing) {
      throw new DeployError(`制品源不存在：${requestedId}`, { code: 'ARTIFACT_SOURCE_NOT_FOUND' });
    }
    const id = existing?.id || nextArtifactSourceId(sources);
    const record = validateArtifactSource(mergeSourceInput(args, existing), existing);
    if (args.clearToken === true) {
      await removeArtifactToken(id);
    } else if (String(args.token ?? '').trim()) {
      await writeArtifactToken(id, args.token);
    }
    let hasToken = Boolean(await readArtifactToken(id));
    if (args.clearToken === true) hasToken = false;
    else if (String(args.token ?? '').trim()) hasToken = true;
    else if (existing?.hasToken) hasToken = true;
    let saved = null;
    const path = (await mutateArtifactSources((list) => {
      const index = list.findIndex((source) => source.id === id);
      const previous = index >= 0 ? list[index] : null;
      const next = {
        ...record,
        id,
        hasToken,
        lastVersion: previous?.lastVersion || '',
        lastBundleDir: previous?.lastBundleDir || ''
      };
      if (index >= 0) list[index] = next;
      else list.push(next);
      saved = next;
    })).path;
    return { path, source: publicSource(saved) };
  });

  register('deploy_artifact_source_remove', {
    title: '删除制品源',
    description: '删除制品源及其令牌与版本缓存；已下载的交付包保留在本机制品缓存中。',
    inputSchema: { source: sourceSelector, confirm: z.boolean().optional() },
    annotations: destructive
  }, async (args) => {
    if (args.confirm !== true) {
      throw new DeployError('删除制品源会同时移除令牌与版本缓存，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    }
    const source = await resolveSource(args.source);
    const path = (await mutateArtifactSources((list) => {
      const index = list.findIndex((item) => item.id === source.id);
      if (index < 0) throw new DeployError(`制品源不存在：${source.id}`, { code: 'ARTIFACT_SOURCE_NOT_FOUND' });
      list.splice(index, 1);
    })).path;
    await removeArtifactToken(source.id);
    await removeReleaseCacheEntry(source.id);
    return { path, removed: { id: source.id, name: source.name } };
  });

  register('deploy_artifact_releases', {
    title: '查看制品版本',
    description: '读取制品源中可下载的版本与运行信息。可只查看已缓存版本，不访问网络。',
    inputSchema: {
      source: sourceSelector,
      page: z.number().int().min(1).optional().default(1).describe('云效运行列表页码'),
      cachedOnly: z.boolean().optional().default(false).describe('只读取本机已缓存的版本与已下载交付包'),
      token: z.string().optional().describe('本次查询使用的云效令牌；不会保存')
    },
    annotations: remoteReadOnly
  }, async (args) => {
    const source = await resolveSource(args.source);
    const result = await listArtifactReleases(source, {
      page: args.page,
      cachedOnly: args.cachedOnly,
      token: args.token,
      persist: false
    });
    return { source: publicSource(source), ...result };
  });

  register('deploy_artifact_download', {
    title: '下载交付包',
    description: '下载指定版本的交付包，校验 MD5、版本与文件清单后保存到本机制品缓存。下载完成后可再绑定到项目。',
    inputSchema: {
      source: sourceSelector,
      version: z.string().optional().describe('要下载的版本；云效源留空时取最新成功运行，直接下载源使用源中配置的版本'),
      runId: z.number().int().min(1).optional().describe('云效流水线运行 ID；优先于版本搜索'),
      token: z.string().optional().describe('本次下载使用的云效令牌；不会保存')
    },
    annotations: mutating
  }, async (args, extra) => {
    const source = await resolveSource(args.source);
    const release = await resolveArtifactRelease(source, {
      version: args.version,
      runId: args.runId,
      token: args.token
    });
    const progress = progressReporter(extra);
    const result = await downloadArtifact(source, release, {
      onProgress: (event) => {
        const total = Number(event.total) || 0;
        const downloaded = Number(event.downloaded) || 0;
        if (event.stage === 'download') {
          const percent = total > 0 ? Math.min(99, Math.round((downloaded / total) * 100)) : 0;
          progress(percent, 100, `正在下载交付包：${formatBytes(downloaded)} / ${formatBytes(total)}`);
          return;
        }
        if (event.stage === 'extract') progress(100, 100, '正在校验并解压交付包');
      }
    });
    progress(100, 100, '交付包已保存');
    return {
      source: publicSource(source),
      dir: result.dir,
      bundleDir: result.dir,
      version: result.version,
      commitShort: result.commit,
      files: result.files,
      sizeBytes: result.sizeBytes,
      archiveBytes: result.archiveBytes,
      md5: result.md5,
      hint: '可使用 deploy_project_bind_bundle 把该交付包绑定到项目。'
    };
  });
}
