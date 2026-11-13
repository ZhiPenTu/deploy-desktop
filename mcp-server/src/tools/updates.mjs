import { z } from 'zod';
import { loadConfig, mutateConfig } from '../config.mjs';
import { DeployError } from '../errors.mjs';
import { appVersion, loadProfile } from '../profile.mjs';
import { checkAppUpdate } from '../update.mjs';
import { Bundle } from '../bundle.mjs';
import { activeProject } from '../projects.mjs';
import {
  assertFullComponents,
  deployVersionRemote,
  inspectRemoteTransaction,
  recoverVersionRemote
} from '../version-deploy.mjs';
import { hostSchema, remoteFields, remoteReadOnly, mutating, destructive, hostFrom } from './shared.mjs';
import { progressReporter } from './runtime.mjs';

export function registerUpdatesTools(register) {
  register('deploy_version_remote', {
    title: '部署完整版本',
    description: '停止目标服务，备份受管文件与数据卷，部署交付包声明的全部组件并验证就绪；失败时自动恢复上一版本。实际部署需 confirm=true。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      bundleDir: z.string().min(1).describe('交付包目录，需包含 bundle.json'),
      installDir: z.string().optional(),
      dryRun: z.boolean().optional().default(false),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args, extra) => {
    if (!args.dryRun && args.confirm !== true) {
      throw new DeployError('完整版本部署会停止目标服务并替换全部受管文件，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    }
    const { profile } = await loadProfile({ bundleDir: args.bundleDir, installDir: args.installDir });
    const host = await hostFrom(args);
    const progress = progressReporter(extra);
    return deployVersionRemote(profile, host, {
      bundleDir: args.bundleDir,
      installDir: args.installDir,
      dryRun: args.dryRun,
      onProgress: (percent, message) => progress(percent, 100, message)
    });
  });

  register('deploy_transaction_status', {
    title: '查询部署事务',
    description: '读取目标机最新部署事务的阶段、状态、快照目录和失败原因；不修改目标。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      installDir: z.string().optional()
    },
    annotations: remoteReadOnly
  }, async (args) => {
    const host = await hostFrom(args);
    let installDir = String(args.installDir || host.installDir || '').trim();
    if (!installDir) {
      try {
        const { project } = await activeProject();
        installDir = String(project.installDir || '').trim();
      } catch (error) {
        if (error?.code !== 'PROJECT_NOT_FOUND') throw error;
      }
    }
    if (!installDir) throw new DeployError('请提供部署目录', { code: 'INSTALL_DIR_REQUIRED' });
    return { transaction: await inspectRemoteTransaction(host, installDir) };
  });

  register('deploy_transaction_recover', {
    title: '恢复上一版本',
    description: '按部署事务快照恢复部署前的受管文件、数据卷并启动上一版本。恢复失败后不会自动重试，需要 confirm=true。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      installDir: z.string().optional(),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args, extra) => {
    if (args.confirm !== true) {
      throw new DeployError('恢复上一版本会停止当前服务并覆盖部署目录，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    }
    const { project } = await activeProject();
    const host = await hostFrom(args);
    const progress = progressReporter(extra);
    return recoverVersionRemote(
      { projectId: project.id, installDir: args.installDir || project.installDir },
      host,
      {
        installDir: args.installDir,
        onProgress: (percent, message) => progress(percent, 100, message)
      }
    );
  });

  register('deploy_update_remote', {
    title: '更新远程部署',
    description: '兼容旧调用的完整版本部署别名；只接受交付包全部组件，部分组件更新会被拒绝。实际部署需 confirm=true。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      bundleDir: z.string().min(1).describe('交付包目录，需包含 bundle.json'),
      installDir: z.string().optional(),
      components: z.array(z.string()).optional().describe('兼容参数；传入时必须是交付包全部组件'),
      backup: z.boolean().optional().describe('兼容参数；完整版本部署始终备份，backup=false 会被拒绝'),
      dryRun: z.boolean().optional().default(false),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args, extra) => {
    if (!args.dryRun && args.confirm !== true) {
      throw new DeployError('实际部署会替换目标机文件并重启服务，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    }
    if (args.backup === false) {
      throw new DeployError('当前版本已改为完整版本部署，不支持关闭部署前备份', {
        code: 'FULL_DEPLOY_BACKUP_REQUIRED'
      });
    }
    const { profile } = await loadProfile({ bundleDir: args.bundleDir, installDir: args.installDir });
    const bundle = await Bundle.load(args.bundleDir, { appVersion: await appVersion() });
    if (args.components !== undefined) assertFullComponents(bundle, args.components);
    const host = await hostFrom(args);
    const progress = progressReporter(extra);
    return deployVersionRemote(profile, host, {
      bundleDir: args.bundleDir,
      installDir: args.installDir,
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
