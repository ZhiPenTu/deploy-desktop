import { updateLocalDeployment, updateRemoteDeployment } from '../../../api/tauri.js';
import { requireSuccessfulResult } from '../domain/components.js';

// Only the immutable context captured before confirmation reaches the executor.
export async function invokeIncremental(context, dryRun) {
  const response = context.remote
    ? await updateRemoteDeployment(context.host, context.port, context.user, context.keyPath,
      context.bundleDir, context.installDir, [...context.components], context.backup, dryRun)
    : await updateLocalDeployment(context.bundleDir, context.installDir,
      [...context.components], context.backup, dryRun);
  return requireSuccessfulResult(response, dryRun ? '增量更新预检未通过' : '更新未完成');
}
