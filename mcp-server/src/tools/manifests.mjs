import { z } from 'zod';
import { loadManifestSummary, writeManifest } from '../manifest.mjs';
import { remoteReadOnly, mutating, manifestOverrides } from './shared.mjs';

export function registerManifestsTools(register) {
  register('deploy_manifest_load', {
    title: '加载并解析镜像清单',
    description: '从 HTTP(S) 或本地文件加载 deploy.desktop/v1 YAML/JSON 清单，校验 digest 并解析完整镜像引用。',
    inputSchema: {
      source: z.string().min(1).describe('清单 URL 或本地路径'),
      registry: z.string().optional().describe('覆盖清单中的 spec.registry；可填仓库配置名或 registry 主机名'),
      namespace: z.string().optional().describe('覆盖清单中的 spec.namespace')
    },
    annotations: remoteReadOnly
  }, async (args) => loadManifestSummary(args.source, await manifestOverrides(args)));

  register('deploy_manifest_validate', {
    title: '校验镜像清单',
    description: '校验清单结构、digest、repository、platform 与未知字段，不拉取镜像。',
    inputSchema: {
      source: z.string().min(1),
      registry: z.string().optional().describe('仓库配置名或 registry 主机名'),
      namespace: z.string().optional()
    },
    annotations: remoteReadOnly
  }, async (args) => {
    const summary = await loadManifestSummary(args.source, await manifestOverrides(args));
    return { valid: true, source: summary.source, images: summary.images, warnings: summary.warnings };
  });

  register('deploy_manifest_write', {
    title: '写入镜像清单',
    description: '校验并写入 deploy.desktop/v1 清单文件（YAML）。覆盖已有文件需 overwrite=true。',
    inputSchema: {
      file: z.string().min(1).describe('目标文件路径'),
      manifest: z.record(z.string(), z.any()).describe('完整清单对象'),
      overwrite: z.boolean().optional().default(false)
    },
    annotations: mutating
  }, async (args) => writeManifest(args.file, args.manifest, { overwrite: args.overwrite }));
}
