export const componentLabels = Object.freeze({
  image: '镜像', backend: '后端服务', gateway: '网关服务', frontend: '前端页面', config: '部署配置'
});

export function deploymentComponents(plan) {
  const declared = Array.isArray(plan?.components) ? plan.components : [];
  const keys = plan?.mode === 'image'
    ? ['image', 'config']
    : declared.length ? declared : ['backend', 'gateway', 'frontend', 'config'];
  return [...new Set(keys)].filter((key) => Object.hasOwn(componentLabels, key) && (plan?.mode !== 'file' || key !== 'image'));
}

export function servicesAreReady(review) {
  const services = Array.isArray(review?.services)
    ? review.services.filter((service) => service.change !== 'removed')
    : [];
  return Boolean(review?.targetChecked && services.length && services.every((service) =>
    service.containerState === 'running' && !['unhealthy', 'starting'].includes(service.containerHealth)
  ));
}

export function requireSuccessfulResult(result, message) {
  if (result?.success !== true) {
    const error = new Error(result?.error || message);
    error.result = result;
    throw error;
  }
  return result;
}
