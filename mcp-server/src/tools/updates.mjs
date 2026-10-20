import { z } from 'zod';
import { loadConfig, mutateConfig } from '../config.mjs';
import { DeployError } from '../errors.mjs';
import { appVersion, loadProfile } from '../profile.mjs';
import { checkAppUpdate, normalizeComponents, updateRemote } from '../update.mjs';
import { hostSchema, remoteFields, remoteReadOnly, mutating, destructive, hostFrom } from './shared.mjs';
import { progressReporter } from './runtime.mjs';

export function registerUpdatesTools(register) {
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
