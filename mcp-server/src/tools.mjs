import { z } from 'zod';
import { loadConfig, mutateConfig, saveConfig, validateHost, validateRegistry, ensureDefaultRegistry } from './config.mjs';
import { DeployError } from './errors.mjs';
import {
  appVersion,
  containerName,
  databaseContainer,
  loadProfile,
  updateProfile,
  validateProfile
} from './profile.mjs';
import {
  checkLocalEnv,
  checkRemoteEnv,
  composeAction,
  dbCheck,
  dbImport,
  inspectLocalDeployment,
  inspectRemoteDeployment,
  installDocker,
  runRemoteDeploy,
  serviceLogs
} from './deploy.mjs';
import {
  inspectRemoteBundle,
  listLocalImages,
  listRemoteImages,
  loadLocalImages,
  loadRemoteImages,
  pullImages,
  removeImages,
  removeRemoteBundleFiles,
  resolveImagesDir,
  saveImages,
  scanLocalBundle,
  uploadBundle
} from './images.mjs';
import { loadManifestSummary, selectImages, validateManifest, writeManifest } from './manifest.mjs';
import {
  dockerLoginLocal,
  dockerLoginRemote,
  dockerLogoutLocal,
  dockerLogoutRemote,
  listTags,
  probeRegistry,
  registryFromArgs,
  resolveDigest
} from './registry.mjs';
import { resolveHost, setupSshKey, testSshConnection } from './ssh.mjs';
import { checkAppUpdate, normalizeComponents, updateRemote } from './update.mjs';
import { expandHome } from './util.mjs';

const targetSchema = z.enum(['local', 'remote']).default('local').describe('local=当前机器，remote=通过 SSH 操作目标机');
const hostSchema = z.string().min(1).describe('SSH 主机配置名（deploy_host_save 保存的名称）或 IP/域名');
const remoteFields = {
  host: hostSchema.optional(),
  port: z.number().int().min(1).max(65535).optional().describe('SSH 端口，默认 22'),
  user: z.string().optional().describe('SSH 用户，默认 root'),
  keyPath: z.string().optional().describe('SSH 私钥路径，默认自动查找 ~/.ssh/id_ed25519 等')
};
const registryFields = {
  registry: z.string().optional().describe('仓库配置名或 registry 主机名'),
  username: z.string().optional(),
  password: z.string().optional().describe('仓库密码；仅用于本次调用，不会写入磁盘'),
  passwordEnv: z.string().optional().describe('仓库密码所在环境变量名'),
  insecure: z.boolean().optional().describe('允许使用 HTTP registry')
};
const readOnly = { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false };
const remoteReadOnly = { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: true };
const mutating = { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: true };
const destructive = { readOnlyHint: false, destructiveHint: true, idempotentHint: false, openWorldHint: true };

function result(value) {
  return {
    content: [{ type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value, null, 2) }]
  };
}

function errorResult(error) {
  const payload = {
    error: error?.message || String(error),
    code: error?.code || 'DEPLOY_ERROR'
  };
  if (error?.hint) payload.hint = error.hint;
  if (error?.details) payload.details = error.details;
  return { content: [{ type: 'text', text: JSON.stringify(payload, null, 2) }], isError: true };
}

function wrap(handler) {
  return async (args, extra) => {
    try {
      return result(await handler(args, extra));
    } catch (error) {
      return errorResult(error);
    }
  };
}

function progressReporter(extra) {
  const token = extra?._meta?.progressToken;
  return async (current, total, message) => {
    if (token === undefined) return;
    try {
      await extra.sendNotification({
        method: 'notifications/progress',
        params: { progressToken: token, progress: current, total, message }
      });
    } catch {
      // Progress is best-effort and must not fail the tool call.
    }
  };
}

async function hostFrom(args) {
  const { host } = await resolveHost(args.host, args);
  return host;
}

async function manifestOverrides(args) {
  const { config } = await loadConfig();
  const saved = args.registry ? config.registries[args.registry] : null;
  return {
    registryOverride: saved ? saved.url : args.registry,
    namespaceOverride: args.namespace !== undefined ? args.namespace : (saved?.namespace || undefined)
  };
}

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

export function registerTools(server) {
  const register = (name, definition, handler) => {
    server.registerTool(name, definition, wrap(handler));
  };

  register('deploy_profile_get', {
    title: '读取部署配置',
    description: '读取当前生效的 profile.json（容器前缀、服务端口、数据库、镜像包清单）及来源路径。',
    inputSchema: {},
    annotations: readOnly
  }, async () => {
    const { profile, source } = await loadProfile();
    return { profile, source, appVersion: await appVersion() };
  });

  register('deploy_profile_update', {
    title: '修改部署配置',
    description: '更新 profile.json 中的容器、端口、数据库或镜像包配置。默认只预览；写入必须 confirm=true。',
    inputSchema: {
      patch: z.record(z.string(), z.any()).describe('要修改的字段，例如 {"containerPrefix":"tt-net-manager","containers":[...]}'),
      dryRun: z.boolean().optional().default(true).describe('true 只返回变更预览，不写文件'),
      confirm: z.boolean().optional().describe('写入文件时必须为 true')
    },
    annotations: destructive
  }, async (args) => updateProfile(args.patch, { dryRun: args.dryRun !== false, confirm: args.confirm }));

  register('deploy_host_list', {
    title: '列出 SSH 主机配置',
    description: '列出 ~/.deploy-desktop/mcp.json 中保存的 SSH 主机配置（不包含密码）。',
    inputSchema: {},
    annotations: readOnly
  }, async () => {
    const { config, path } = await loadConfig();
    return { path, hosts: Object.values(config.hosts) };
  });

  register('deploy_host_save', {
    title: '保存 SSH 主机配置',
    description: '新增或更新一个 SSH 主机配置。密码不会保存，请使用密钥登录。',
    inputSchema: {
      name: z.string().min(1).describe('配置名，后续工具可用 name 代替 IP'),
      host: z.string().min(1),
      port: z.number().int().min(1).max(65535).optional().default(22),
      user: z.string().optional().default('root'),
      keyPath: z.string().optional(),
      installDir: z.string().optional().describe('默认 /opt/deploy'),
      deployDir: z.string().optional().describe('默认 <installDir>/deploy'),
      bundleDir: z.string().optional().describe('默认 /opt/deploy/bundle'),
      strictHostKeyChecking: z.enum(['yes', 'accept-new', 'no']).optional().default('accept-new')
    },
    annotations: mutating
  }, async (args) => {
    const host = validateHost(args);
    const { path } = await mutateConfig((config) => {
      config.hosts[host.name] = host;
    });
    return { path, host };
  });

  register('deploy_host_remove', {
    title: '删除 SSH 主机配置',
    description: '删除保存的 SSH 主机配置，不会修改目标机。',
    inputSchema: { name: z.string().min(1), confirm: z.boolean().optional() },
    annotations: destructive
  }, async (args) => {
    if (args.confirm !== true) throw new DeployError('删除主机配置需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    let removed = null;
    const { path } = await mutateConfig((config) => {
      removed = config.hosts[args.name] || null;
      delete config.hosts[args.name];
    });
    return { path, removed };
  });

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

  register('deploy_ssh_test', {
    title: '测试 SSH 连接',
    description: '测试到目标机的 SSH 密钥登录并返回往返时延。',
    inputSchema: { ...remoteFields, host: hostSchema },
    annotations: remoteReadOnly
  }, async (args) => testSshConnection(await hostFrom(args)));

  register('deploy_ssh_setup_key', {
    title: '配置免密 SSH 密钥',
    description: '生成 ED25519 密钥并通过初始密码下发公钥；没有 sshpass 时返回公钥供手动安装。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      password: z.string().optional().describe('目标机初始密码；不会被保存')
    },
    annotations: mutating
  }, async (args) => setupSshKey(await hostFrom(args), args.password || ''));

  register('deploy_env_check', {
    title: '环境自检',
    description: '检查 CPU、内存、磁盘、Docker、Compose 与 profile 中声明的端口占用。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      targetDir: z.string().optional().describe('本地磁盘检查路径')
    },
    annotations: remoteReadOnly
  }, async (args) => {
    const { profile } = await loadProfile();
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      return checkRemoteEnv(host, profile);
    }
    return checkLocalEnv(profile, args.targetDir);
  });

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
      bundleDir: z.string().min(1).describe('镜像包根目录（读取其 images/ 子目录）'),
      files: z.array(z.string()).optional().describe('指定文件名；为空表示全部')
    },
    annotations: mutating
  }, async (args) => {
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      return { target: 'remote', loaded: await loadRemoteImages(host, args.bundleDir, args.files || []) };
    }
    return { target: 'local', loaded: await loadLocalImages(args.bundleDir, args.files || []) };
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

  register('deploy_bundle_scan', {
    title: '扫描离线镜像包',
    description: '扫描本地或远程 <bundleDir>/images 下的 tar/tgz 文件、总大小与 profile 期望文件缺口。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      dir: z.string().optional().describe('本地镜像包目录'),
      bundleDir: z.string().optional().describe('远程镜像包根目录')
    },
    annotations: remoteReadOnly
  }, async (args) => {
    const { profile } = await loadProfile();
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      return inspectRemoteBundle(profile, host, args.bundleDir || profile.bundleDir);
    }
    return scanLocalBundle(profile, args.dir || profile.bundleDir);
  });

  register('deploy_bundle_upload', {
    title: '上传离线镜像包',
    description: '把本地镜像包上传到目标机 <remoteBundleDir>/images；默认跳过大小一致的文件。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      localDir: z.string().min(1).describe('本地镜像包目录或 images 目录'),
      remoteBundleDir: z.string().min(1),
      files: z.array(z.string()).optional(),
      overwrite: z.boolean().optional().default(false)
    },
    annotations: mutating
  }, async (args) => {
    const host = await hostFrom(args);
    const { imagesDir } = await resolveImagesDir(args.localDir);
    const messages = await uploadBundle(host, args.localDir, args.remoteBundleDir, args.files || [], { overwrite: args.overwrite });
    return { localDir: imagesDir, remoteDir: `${args.remoteBundleDir.replace(/\/+$/, '')}/images`, messages };
  });

  register('deploy_bundle_import', {
    title: '导入目标机离线镜像',
    description: '在目标机对 <remoteBundleDir>/images 中的镜像包执行 docker load。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      remoteBundleDir: z.string().min(1),
      files: z.array(z.string()).optional()
    },
    annotations: mutating
  }, async (args) => {
    const host = await hostFrom(args);
    return { loaded: await loadRemoteImages(host, args.remoteBundleDir, args.files || []) };
  });

  register('deploy_bundle_deploy', {
    title: '一键离线部署',
    description: '串联离线镜像上传/导入、Compose 启动与可选数据库导入，结束后返回部署状态。首次部署或整机重建使用，需要 confirm=true。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      bundleDir: z.string().min(1).describe('本地镜像包根目录（读取其 images/ 子目录）'),
      remoteBundleDir: z.string().optional().describe('远程镜像包根目录；默认使用主机配置或 profile.bundleDir'),
      files: z.array(z.string()).optional().describe('只处理指定镜像包文件名；为空表示全部'),
      upload: z.boolean().optional().default(true).describe('远程模式先上传本地镜像包'),
      service: z.string().optional().describe('只启动指定服务，默认全部'),
      sqlFile: z.string().optional().describe('启动成功后可选导入的本地 SQL 文件'),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args) => {
    if (args.confirm !== true) {
      throw new DeployError('一键部署会导入镜像并启动服务，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    }
    const { profile } = await loadProfile();
    const files = args.files || [];
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      const remoteBundleDir = args.remoteBundleDir || host.bundleDir;
      let uploads = [];
      if (args.upload !== false) {
        uploads = await uploadBundle(host, args.bundleDir, remoteBundleDir, files, { overwrite: false });
      }
      const loaded = await loadRemoteImages(host, remoteBundleDir, files);
      const compose = await composeAction(profile, {
        target: 'remote',
        hostConfig: host,
        action: 'up',
        service: args.service,
        confirm: true
      });
      const database = args.sqlFile
        ? await dbImport(profile, { target: 'remote', hostConfig: host, sqlFile: args.sqlFile, confirm: true })
        : null;
      const status = await inspectRemoteDeployment(profile, host, host.installDir).catch(() => null);
      return { target: 'remote', remoteBundleDir, uploads, loaded, compose, database, status };
    }
    const loaded = await loadLocalImages(args.bundleDir, files);
    const compose = await composeAction(profile, {
      target: 'local',
      action: 'up',
      service: args.service,
      confirm: true
    });
    const database = args.sqlFile
      ? await dbImport(profile, { target: 'local', sqlFile: args.sqlFile, confirm: true })
      : null;
    const status = await inspectLocalDeployment(profile).catch(() => null);
    return { target: 'local', loaded, compose, database, status };
  });

  register('deploy_bundle_remove', {
    title: '删除目标机镜像包文件',
    description: '删除目标机 <remoteBundleDir>/images 下的指定文件，不删除已导入的 Docker 镜像。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      remoteBundleDir: z.string().min(1),
      files: z.array(z.string()).min(1),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args) => {
    if (args.confirm !== true) throw new DeployError('删除镜像包文件需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    const host = await hostFrom(args);
    return removeRemoteBundleFiles(host, args.remoteBundleDir, args.files);
  });

  register('deploy_compose', {
    title: '执行 Compose 操作',
    description: '在本地或远程部署目录执行 docker compose up/down/stop/restart/ps/pull。down 需要 confirm=true。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      action: z.enum(['up', 'down', 'stop', 'restart', 'ps', 'pull']),
      service: z.string().optional().describe('服务名或容器名；为空表示全部'),
      installDir: z.string().optional(),
      deployDir: z.string().optional(),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args) => {
    const { profile } = await loadProfile();
    const host = args.target === 'remote' ? await hostFrom(args) : null;
    return composeAction(profile, {
      target: args.target,
      hostConfig: host,
      action: args.action,
      service: args.service,
      deployDir: args.deployDir,
      installDir: args.installDir,
      confirm: args.confirm
    });
  });

  register('deploy_inspect', {
    title: '检查部署状态',
    description: '检查本地或远程部署目录、产物文件、Compose 状态与容器健康状态。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      installDir: z.string().optional(),
      deployDir: z.string().optional().describe('本地部署目录；默认 profile.deployDir')
    },
    annotations: remoteReadOnly
  }, async (args) => {
    const { profile } = await loadProfile();
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      return inspectRemoteDeployment(profile, host, args.installDir);
    }
    return inspectLocalDeployment(profile, args.installDir, args.deployDir);
  });

  register('deploy_remote_run', {
    title: '远程构建并部署',
    description: '调用仓库根目录 deploy-lan.sh 执行 deploy-all/server-only/gateway-app-only/frontend-only。需要 confirm=true。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      mode: z.enum(['deploy-all', 'server-only', 'gateway-app-only', 'frontend-only']),
      deployDir: z.string().optional(),
      timeoutSec: z.number().int().min(60).max(7200).optional().default(3600),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args) => {
    const { profile } = await loadProfile();
    const host = await hostFrom(args);
    return runRemoteDeploy(profile, host, {
      mode: args.mode,
      deployDir: args.deployDir,
      timeoutMs: args.timeoutSec * 1000,
      confirm: args.confirm
    });
  });

  register('deploy_logs', {
    title: '读取服务日志',
    description: '读取本地或远程容器最近 N 行日志。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      service: z.string().optional().describe('服务键，例如 backend-java'),
      container: z.string().optional().describe('完整容器名；与 service 二选一'),
      tail: z.number().int().min(1).max(5000).optional().default(200)
    },
    annotations: remoteReadOnly
  }, async (args) => {
    const { profile } = await loadProfile();
    const host = args.target === 'remote' ? await hostFrom(args) : null;
    return serviceLogs(profile, {
      target: args.target,
      hostConfig: host,
      service: args.service,
      container: args.container,
      tail: args.tail
    });
  });

  register('deploy_db_check', {
    title: '检查数据库状态',
    description: '检查 OceanBase 容器与数据库连接是否就绪。',
    inputSchema: { target: targetSchema, ...remoteFields },
    annotations: remoteReadOnly
  }, async (args) => {
    const { profile } = await loadProfile();
    const host = args.target === 'remote' ? await hostFrom(args) : null;
    return dbCheck(profile, { target: args.target, hostConfig: host });
  });

  register('deploy_db_import', {
    title: '导入数据库 SQL',
    description: '在本地或远程 OceanBase 容器中创建数据库并导入 SQL；远程模式可自动上传本地 SQL 文件。会写入业务数据，需要 confirm=true。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      sqlFile: z.string().min(1).describe('本地 SQL 文件路径'),
      remoteSqlPath: z.string().optional().describe('远程模式使用目标机已有 SQL 路径，避免重复上传'),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args) => {
    if (args.confirm !== true) {
      throw new DeployError('导入数据库会写入业务数据，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    }
    const { profile } = await loadProfile();
    const host = args.target === 'remote' ? await hostFrom(args) : null;
    return dbImport(profile, {
      target: args.target,
      hostConfig: host,
      sqlFile: args.sqlFile,
      remoteSqlPath: args.remoteSqlPath,
      confirm: true
    });
  });

  register('deploy_docker_install', {
    title: '离线安装 Docker',
    description: '从 <bundleDir>/docker-engine 离线安装 Docker 与 Compose。会修改系统文件，需要 confirm=true。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      bundleDir: z.string().optional(),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args) => {
    const { profile } = await loadProfile();
    const host = args.target === 'remote' ? await hostFrom(args) : null;
    return installDocker(profile, { target: args.target, hostConfig: host, bundleDir: args.bundleDir, confirm: args.confirm });
  });

  register('deploy_update_remote', {
    title: '更新远程部署',
    description: '上传 app/ 与 deploy/ 产物，按组件替换并重启服务。支持预检与更新前备份。实际更新需 confirm=true。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      bundleDir: z.string().min(1).describe('更新包目录，包含 app/ 与 deploy/'),
      installDir: z.string().optional(),
      components: z.array(z.enum(['backend', 'gateway', 'frontend', 'config'])).min(1),
      backup: z.boolean().optional().default(true),
      dryRun: z.boolean().optional().default(false),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args, extra) => {
    if (!args.dryRun && args.confirm !== true) {
      throw new DeployError('实际更新会替换目标机文件并重启服务，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    }
    const { profile } = await loadProfile();
    const host = await hostFrom(args);
    const progress = progressReporter(extra);
    return updateRemote(profile, host, {
      bundleDir: args.bundleDir,
      installDir: args.installDir,
      components: normalizeComponents(args.components),
      backup: args.backup,
      dryRun: args.dryRun,
      onProgress: (percent, message) => progress(percent, 100, message)
    });
  });

  register('deploy_update_settings', {
    title: '读取或保存应用更新源',
    description: '读取或保存可选的 Tauri updater 自定义地址；默认直接使用 GitHub Releases，无需配置。保存需要 confirm=true。',
    inputSchema: {
      endpoint: z.string().optional(),
      allowInsecure: z.boolean().optional(),
      confirm: z.boolean().optional()
    },
    annotations: mutating
  }, async (args) => {
    const { config, path } = await loadConfig();
    if (args.endpoint === undefined && args.allowInsecure === undefined) {
      return { path, update: config.update };
    }
    if (args.confirm !== true) throw new DeployError('保存更新源设置需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    if (args.endpoint !== undefined) {
      const endpoint = String(args.endpoint).trim();
      if (endpoint && !/^https?:\/\//i.test(endpoint)) throw new DeployError('更新源地址必须以 http:// 或 https:// 开头');
      if (endpoint && !/^https:\/\//i.test(endpoint) && !(args.allowInsecure ?? config.update.allowInsecure)) {
        throw new DeployError('HTTP 更新源需要同时设置 allowInsecure=true');
      }
    }
    const { config: next } = await mutateConfig((draft) => {
      if (args.endpoint !== undefined) draft.update.endpoint = String(args.endpoint).trim();
      if (args.allowInsecure !== undefined) draft.update.allowInsecure = Boolean(args.allowInsecure);
    });
    return { path, update: next.update };
  });

  register('deploy_app_update_check', {
    title: '检查应用更新',
    description: '默认从 GitHub Releases 读取 latest.json，比较版本并返回当前平台下载信息；不会自动安装。',
    inputSchema: {
      endpoint: z.string().optional().describe('latest.json 地址；留空则使用 MCP 配置的 update.endpoint，再回退到 GitHub Releases'),
      allowInsecure: z.boolean().optional().default(false),
      currentVersion: z.string().optional()
    },
    annotations: remoteReadOnly
  }, async (args) => {
    const { config } = await loadConfig();
    const endpoint = args.endpoint || config.update?.endpoint;
    return checkAppUpdate({
      endpoint,
      allowInsecure: args.allowInsecure || config.update?.allowInsecure,
      currentVersion: args.currentVersion || await appVersion()
    });
  });
}
