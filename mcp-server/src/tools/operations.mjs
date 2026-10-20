import { z } from 'zod';
import { DeployError } from '../errors.mjs';
import { loadProfile } from '../profile.mjs';
import { composeAction, dbCheck, dbImport, inspectLocalDeployment, inspectRemoteDeployment, installDocker, runRemoteDeploy, serviceLogs } from '../deploy.mjs';
import { targetSchema, hostSchema, remoteFields, remoteReadOnly, destructive, hostFrom } from './shared.mjs';

export function registerOperationsTools(register) {
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
}
