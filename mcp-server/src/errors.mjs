export class DeployError extends Error {
  constructor(message, options = {}) {
    super(message);
    this.name = 'DeployError';
    this.code = options.code || 'DEPLOY_ERROR';
    this.hint = options.hint;
    this.details = options.details;
  }
}

export function fail(message, options) {
  throw new DeployError(message, options);
}

export function toErrorPayload(error) {
  return {
    error: error?.message || String(error),
    code: error?.code || 'DEPLOY_ERROR',
    hint: error?.hint || undefined,
    details: error?.details || undefined
  };
}
