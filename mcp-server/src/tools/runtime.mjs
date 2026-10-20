import { toErrorPayload } from '../errors.mjs';

export function wrapTool(handler) {
  return async (args, extra) => {
    try {
      const value = await handler(args, extra);
      return { content: [{ type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value, null, 2) }] };
    } catch (error) {
      return { content: [{ type: 'text', text: JSON.stringify(toErrorPayload(error), null, 2) }], isError: true };
    }
  };
}

export function progressReporter(extra) {
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
