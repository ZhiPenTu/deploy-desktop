import { z } from 'zod';
import { loadConfig, mutateConfig, validateRegistry, ensureDefaultRegistry } from '../config.mjs';
import { DeployError } from '../errors.mjs';
import { dockerLoginLocal, dockerLoginRemote, dockerLogoutLocal, dockerLogoutRemote, listTags, probeRegistry, registryFromArgs, resolveDigest } from '../registry.mjs';
import { targetSchema, remoteFields, registryFields, readOnly, remoteReadOnly, mutating, destructive, hostFrom } from './shared.mjs';

export function registerRegistriesTools(register) {
  register('deploy_registry_list', {
    title: '列出镜像仓库配置',
    description: '列出保存的镜像仓库配置；密码仅以环境变量名引用，不会返回明文。',
    inputSchema: {},
    annotations: readOnly
  }, async () => {
    const { config, path } = await loadConfig();
    return { path, registries: Object.values(config.registries) };
  });

  register('deploy_registry_save', {
    title: '保存镜像仓库配置',
    description: '新增或更新镜像仓库（ACR、Harbor、GHCR 等）。可保存用户名与密码环境变量名，不保存明文密码。',
    inputSchema: {
      name: z.string().min(1),
      url: z.string().min(1).describe('registry 主机名，例如 registry.cn-hangzhou.aliyuncs.com'),
      namespace: z.string().optional(),
      username: z.string().optional(),
      passwordEnv: z.string().optional().describe('保存密码的环境变量名，例如 ACR_PASSWORD'),
      insecure: z.boolean().optional().default(false),
      isDefault: z.boolean().optional().default(false)
    },
    annotations: mutating
  }, async (args) => {
    const registry = validateRegistry(args);
    const { path } = await mutateConfig((config) => {
      config.registries[registry.name] = registry;
      if (registry.isDefault) ensureDefaultRegistry(config, registry.name);
    });
    return { path, registry };
  });

  register('deploy_registry_remove', {
    title: '删除镜像仓库配置',
    description: '删除保存的镜像仓库配置。',
    inputSchema: { name: z.string().min(1), confirm: z.boolean().optional() },
    annotations: destructive
  }, async (args) => {
    if (args.confirm !== true) throw new DeployError('删除仓库配置需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    let removed = null;
    const { path } = await mutateConfig((config) => {
      removed = config.registries[args.name] || null;
      delete config.registries[args.name];
    });
    return { path, removed };
  });

  register('deploy_registry_probe', {
    title: '检测镜像仓库连通性',
    description: '请求 registry 的 /v2/ 接口，验证网络、TLS 与认证配置。',
    inputSchema: { ...registryFields },
    annotations: remoteReadOnly
  }, async (args) => probeRegistry(await registryFromArgs(args)));

  register('deploy_registry_tags', {
    title: '列出镜像 tag',
    description: '通过 Registry V2 API 列出某个 repository 的 tag。',
    inputSchema: {
      repository: z.string().min(1).describe('例如 your-namespace/backend'),
      limit: z.number().int().min(1).max(1000).optional().default(100),
      ...registryFields
    },
    annotations: remoteReadOnly
  }, async (args) => listTags(await registryFromArgs(args), args.repository, { limit: args.limit }));

  register('deploy_registry_digest', {
    title: '解析镜像 digest',
    description: '通过 Registry V2 API 读取 tag 对应的 sha256 digest，用于生成 deploy.desktop/v1 清单。',
    inputSchema: {
      repository: z.string().min(1).describe('例如 your-namespace/backend'),
      tag: z.string().optional().default('latest'),
      ...registryFields
    },
    annotations: remoteReadOnly
  }, async (args) => resolveDigest(await registryFromArgs(args), args.repository, args.tag));

  register('deploy_registry_login', {
    title: '登录镜像仓库',
    description: '在本地或远程目标机执行 docker login。密码仅用于本次调用。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      ...registryFields
    },
    annotations: mutating
  }, async (args) => {
    const registryConfig = await registryFromArgs(args);
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      return dockerLoginRemote(host, registryConfig);
    }
    return dockerLoginLocal(registryConfig);
  });

  register('deploy_registry_logout', {
    title: '退出镜像仓库',
    description: '在本地或远程目标机执行 docker logout。',
    inputSchema: { target: targetSchema, ...remoteFields, ...registryFields },
    annotations: mutating
  }, async (args) => {
    const registryConfig = await registryFromArgs(args);
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      return dockerLogoutRemote(host, registryConfig);
    }
    return dockerLogoutLocal(registryConfig);
  });
}
