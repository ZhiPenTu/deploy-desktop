import { z } from 'zod';
import { DeployError } from '../errors.mjs';
import { loadProfile } from '../profile.mjs';
import { listLocalImages, listRemoteImages, loadLocalImages, loadRemoteImages, pullImages, removeImages, saveImages } from '../images.mjs';
import { loadManifestSummary, selectImages, validateManifest } from '../manifest.mjs';
import { registryFromArgs } from '../registry.mjs';
import { targetSchema, remoteFields, registryFields, remoteReadOnly, mutating, destructive, hostFrom, manifestOverrides } from './shared.mjs';
import { progressReporter } from './runtime.mjs';

function rawReferencesToImages(references, platform) {
  return references.map((raw, index) => {
    const reference = String(raw).trim();
    if (!reference) throw new DeployError(`references[${index}] 为空`);
    const digestMatch = reference.match(/@sha256:[0-9a-f]{64}$/);
    const name = reference.split('/').pop().split('@')[0].replace(/:[^:]*$/, '');
    return {
      name: name || `image-${index + 1}`,
      reference,
      digest: digestMatch ? digestMatch[0].slice(1) : null,
      platform: platform || null,
      sizeBytes: 0
    };
  });
}

export function registerImagesTools(register) {
  register('deploy_image_pull', {
    title: '拉取镜像',
    description: '按清单、内联清单或镜像引用拉取镜像到本地/远程，校验 digest，可选 docker save 导出 tar。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      source: z.string().optional().describe('清单 URL 或本地路径'),
      manifest: z.record(z.string(), z.any()).optional().describe('内联清单对象'),
      images: z.array(z.string()).optional().describe('只拉取清单中的指定镜像名；为空表示全部'),
      references: z.array(z.string()).optional().describe('直接拉取镜像引用，例如 repo:tag 或 repo@sha256:...'),
      platform: z.string().optional().describe('仅 references 模式使用的平台，例如 linux/amd64'),
      exportDir: z.string().optional().describe('拉取后 docker save 的目录'),
      ...registryFields
    },
    annotations: mutating
  }, async (args, extra) => {
    const { profile } = await loadProfile();
    let images;
    let registryConfig;
    if (args.references?.length) {
      images = rawReferencesToImages(args.references, args.platform);
      registryConfig = args.username || args.registry ? await registryFromArgs(args).catch(() => null) : null;
    } else if (args.manifest) {
      const validated = validateManifest(args.manifest, await manifestOverrides(args));
      images = selectImages({ images: validated.images }, args.images || []);
      registryConfig = await registryFromArgs({ ...args, registry: args.registry || validated.registry }).catch(() => null);
    } else if (args.source) {
      const summary = await loadManifestSummary(args.source, await manifestOverrides(args));
      images = selectImages(summary, args.images || []);
      registryConfig = await registryFromArgs({ ...args, registry: args.registry || summary.registry }).catch(() => null);
    } else {
      throw new DeployError('请提供 source、manifest 或 references');
    }
    const host = args.target === 'remote' ? await hostFrom(args) : null;
    const progress = progressReporter(extra);
    const results = await pullImages(profile, registryConfig, images, {
      target: args.target,
      hostConfig: host,
      exportDir: args.exportDir,
      onProgress: (current, total, message) => progress(current, total, message)
    });
    return { target: args.target, results };
  });

  register('deploy_image_save', {
    title: '导出镜像 tar',
    description: '在本地或远程目标机执行 docker save，导出指定镜像为 tar。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      references: z.array(z.string()).min(1),
      dir: z.string().min(1).describe('导出目录')
    },
    annotations: mutating
  }, async (args) => {
    const host = args.target === 'remote' ? await hostFrom(args) : null;
    return { target: args.target, results: await saveImages(args.target, args.references, args.dir, { hostConfig: host }) };
  });

  register('deploy_image_load', {
    title: '导入离线镜像',
    description: '在本地或远程目标机执行 docker load，导入 <bundleDir>/images 下的 tar/tgz 文件。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      bundleDir: z.string().optional().describe('交付包目录（读取其 images/ 子目录）；默认使用当前项目绑定的交付包'),
      files: z.array(z.string()).optional().describe('指定文件名；为空表示全部')
    },
    annotations: mutating
  }, async (args) => {
    const bundleDir = args.bundleDir || (await loadProfile()).profile.bundleDir;
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      return { target: 'remote', loaded: await loadRemoteImages(host, bundleDir, args.files || []) };
    }
    return { target: 'local', loaded: await loadLocalImages(bundleDir, args.files || []) };
  });

  register('deploy_image_list', {
    title: '列出 Docker 镜像',
    description: '列出本地或远程目标机上的 Docker 镜像。',
    inputSchema: { target: targetSchema, ...remoteFields },
    annotations: remoteReadOnly
  }, async (args) => {
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      return { target: 'remote', images: await listRemoteImages(host) };
    }
    return { target: 'local', images: await listLocalImages() };
  });

  register('deploy_image_remove', {
    title: '删除 Docker 镜像',
    description: '在本地或远程目标机删除指定镜像。需要 confirm=true。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      references: z.array(z.string()).min(1),
      force: z.boolean().optional().default(false),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args) => {
    const host = args.target === 'remote' ? await hostFrom(args) : null;
    return removeImages(args.target, args.references, { hostConfig: host, force: args.force, confirm: args.confirm });
  });
}
