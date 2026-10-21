export const IMAGE_COMPONENT = 'image';

/** 可更新组件全部来自交付包声明，客户端不内置组件名称。 */
export function planComponents(plan) {
  const declared = Array.isArray(plan?.components) ? plan.components : [];
  const result = [];
  for (const item of declared) {
    const key = String(item?.key || '').trim();
    if (!key || key === IMAGE_COMPONENT || result.some((entry) => entry.key === key)) continue;
    result.push({ key, label: String(item?.label || '').trim() || key });
  }
  return result;
}

export function hasBundledImages(plan) {
  return (Array.isArray(plan?.images) ? plan.images : [])
    .some((image) => String(image?.bundleFile || '').trim());
}

export function componentOptions(plan) {
  const options = planComponents(plan).map(({ key, label }) => ({ key, label }));
  if (hasBundledImages(plan)) options.push({ key: IMAGE_COMPONENT, label: '随包镜像' });
  return options;
}

export function componentLabels(plan) {
  return Object.fromEntries(componentOptions(plan).map(({ key, label }) => [key, label]));
}

export function deploymentComponents(plan) {
  return componentOptions(plan).map(({ key }) => key);
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
