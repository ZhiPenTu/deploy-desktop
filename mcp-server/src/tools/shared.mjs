import { z } from 'zod';
import { loadConfig } from '../config.mjs';
import { resolveHost } from '../ssh.mjs';

export const targetSchema = z.enum(['local', 'remote']).default('local').describe('local=当前机器，remote=通过 SSH 操作目标机');

export const hostSchema = z.string().min(1).optional().describe('SSH 主机名或 IP/域名；留空使用当前项目绑定的主机');

export const remoteFields = {
  host: hostSchema.optional(),
  port: z.number().int().min(1).max(65535).optional().describe('SSH 端口，默认 22'),
  user: z.string().optional().describe('SSH 用户，默认 root'),
  keyPath: z.string().optional().describe('SSH 私钥路径，默认自动查找 ~/.ssh/id_ed25519 等')
};

export const registryFields = {
  registry: z.string().optional().describe('仓库配置名或 registry 主机名'),
  username: z.string().optional(),
  password: z.string().optional().describe('仓库密码；仅用于本次调用，不会写入磁盘'),
  passwordEnv: z.string().optional().describe('仓库密码所在环境变量名'),
  insecure: z.boolean().optional().describe('允许使用 HTTP registry')
};

export const readOnly = { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false };

export const remoteReadOnly = { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: true };

export const mutating = { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: true };

export const destructive = { readOnlyHint: false, destructiveHint: true, idempotentHint: false, openWorldHint: true };

export async function hostFrom(args) {
  const { host } = await resolveHost(args.host, args);
  return host;
}

export async function manifestOverrides(args) {
  const { config } = await loadConfig();
  const saved = args.registry ? config.registries[args.registry] : null;
  return {
    registryOverride: saved ? saved.url : args.registry,
    namespaceOverride: args.namespace !== undefined ? args.namespace : (saved?.namespace || undefined)
  };
}
