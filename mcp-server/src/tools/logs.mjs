import { z } from 'zod';
import { readOnly } from './shared.mjs';
import { readRuntimeLogs, runtimeLogSummary } from '../logs.mjs';

export function registerLogsTools(register) {
  register('deploy_runtime_logs_read', {
    title: '读取部署舱运行日志',
    description: '读取本机部署舱的运行日志，可按级别与关键词过滤，最新记录在前。用于排查下载校验、部署与连接失败。',
    inputSchema: {
      level: z.enum(['', 'info', 'warn', 'error', 'debug']).optional().describe('日志级别过滤：留空表示全部，debug 含详细调试记录'),
      keyword: z.string().optional().describe('按内容或来源过滤的关键词'),
      limit: z.number().int().min(1).max(1000).optional().describe('返回条数，默认 100，最大 1000')
    },
    annotations: readOnly
  }, async (args) => readRuntimeLogs({
    level: args.level || '',
    keyword: args.keyword || '',
    limit: args.limit || 100
  }));

  register('deploy_runtime_logs_summary', {
    title: '部署舱运行日志概览',
    description: '汇总本机运行日志的目录、占用大小、各级别条数以及最近一条警告与错误。',
    inputSchema: {},
    annotations: readOnly
  }, async () => runtimeLogSummary());
}
