import { z } from 'zod';
import { loadConfig, mutateConfig, validateHost } from '../config.mjs';
import { DeployError } from '../errors.mjs';
import { loadProfile } from '../profile.mjs';
import { checkLocalEnv, checkRemoteEnv } from '../deploy.mjs';
import { setupSshKey, testSshConnection } from '../ssh.mjs';
import { targetSchema, hostSchema, remoteFields, readOnly, remoteReadOnly, mutating, destructive, hostFrom } from './shared.mjs';

export function registerHostsTools(register) {
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
}
