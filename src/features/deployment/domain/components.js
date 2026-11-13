export function servicesAreReady(review) {
  const services = Array.isArray(review?.services)
    ? review.services.filter((service) => service.change !== 'removed')
    : [];
  const filesReady = !(review?.artifacts || []).some((artifact) =>
    ['missing', 'invalid', 'unconfirmed'].includes(artifact.status)
  );
  return Boolean(review?.targetChecked && filesReady && services.length && services.every((service) =>
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
