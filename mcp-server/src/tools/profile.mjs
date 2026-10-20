import { z } from 'zod';
import { appVersion, loadProfile, updateProfile } from '../profile.mjs';
import { readOnly, destructive } from './shared.mjs';

export function registerProfileTools(register) {
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
}
