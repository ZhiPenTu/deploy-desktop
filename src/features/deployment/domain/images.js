/** 镜像核对状态、原因与按仓库分组的准备操作。 */

export function normalizeRegistry(value) {
  const registry = String(value || '').trim().toLowerCase();
  return registry === 'registry-1.docker.io' || registry === 'index.docker.io'
    ? 'docker.io'
    : registry;
}

export function registryOfReference(reference) {
  const value = String(reference || '');
  const first = value.split('/')[0];
  return normalizeRegistry(
    value.includes('/') && (first.includes('.') || first.includes(':') || first === 'localhost')
      ? first
      : 'docker.io'
  );
}

export function isDigestReference(reference) {
  return String(reference || '').includes('@sha256:');
}

export function imageStatusLabel(status) {
  return (
    {
      ready: '已就绪',
      missing: '缺失',
      platform_mismatch: '架构不匹配',
      check_failed: '检查失败',
      unavailable: '无法检查',
      invalid: '地址无效',
      unconfirmed: '检查失败'
    }[status] || '检查失败'
  );
}

export function imageStatusTone(status) {
  if (status === 'ready') return 'success';
  if (status === 'missing') return 'danger';
  if (status === 'platform_mismatch' || status === 'invalid') return 'warning';
  return 'info';
}

export function imageStatusDetail(image) {
  if (image?.detail) return image.detail;
  const target = image?.targetPlatform || '目标架构';
  switch (image?.status) {
    case 'ready':
      return '目标 Docker 已具备该镜像';
    case 'missing':
      return '目标 Docker 中没有该镜像，可拉取或导入';
    case 'platform_mismatch':
      return isDigestReference(image?.imageRef)
        ? `现有镜像为 ${image?.platform || '未知架构'}，目标为 ${target}；可重新拉取目标架构的版本，若该摘要不含目标架构则需更换交付包`
        : `现有镜像为 ${image?.platform || '未知架构'}，目标为 ${target}；可重新拉取适配架构的版本`;
    case 'unavailable':
      return 'Docker 未运行或目标环境未完成检查';
    case 'invalid':
      return '镜像地址无效，需修正交付包';
    default:
      return '读取镜像状态失败，可重新检查';
  }
}

export function imagePullable(image) {
  if (typeof image?.pullable === 'boolean') return image.pullable;
  return image?.status === 'missing' && !String(image?.imageRef || '').startsWith('-');
}

export function pendingImages(images = []) {
  return images.filter((image) => image.status !== 'ready');
}

export function pullableImages(images = []) {
  return images.filter((image) => image.status !== 'ready' && imagePullable(image));
}

export function imagePreparationCounts(images = []) {
  const count = (status) => images.filter((image) => image.status === status).length;
  return {
    missing: count('missing'),
    mismatch: count('platform_mismatch'),
    failed: images.filter((image) =>
      ['check_failed', 'unavailable', 'unconfirmed'].includes(image.status)
    ).length,
    invalid: count('invalid'),
    total: images.filter((image) => image.status !== 'ready').length
  };
}

export function groupPullReferences(images = [], sources = [], selections = {}) {
  const groups = new Map();
  for (const image of pullableImages(images)) {
    const registry = registryOfReference(image.imageRef);
    if (!groups.has(registry)) {
      groups.set(registry, {
        registry,
        label: registry === 'docker.io' ? 'Docker Hub（docker.io）' : registry,
        references: [],
        sources: sources.filter(
          (source) => normalizeRegistry(source.address) === registry && source.username
        )
      });
    }
    groups.get(registry).references.push(image.imageRef);
  }
  return [...groups.values()].map((group) => {
    const selection = selections[group.registry] || {};
    const source = group.sources.find((item) => item.id === selection.sourceId) || null;
    return {
      ...group,
      count: group.references.length,
      sourceId: source?.id || '',
      hasUsername: Boolean(source?.username),
      needsPassword: Boolean(source?.username && !source?.hasPassword),
      credential: selection.credential || ''
    };
  });
}
