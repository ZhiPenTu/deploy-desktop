import {
  deployLocalVersion,
  deployRemoteVersion
} from '../../../api/tauri.js';
import { requireSuccessfulResult } from '../domain/components.js';

// Only the immutable context captured before confirmation reaches the executor.
export async function invokeVersionDeployment(context, dryRun) {
  const response = context.remote
    ? await deployRemoteVersion(
        context.host,
        context.port,
        context.user,
        context.keyPath,
        context.bundleDir,
        context.installDir,
        dryRun
      )
    : await deployLocalVersion(context.bundleDir, context.installDir, dryRun);
  if (dryRun) {
    return requireSuccessfulResult(response, '完整版本部署预检未通过');
  }
  return response;
}
