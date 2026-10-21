import { projectState } from './project.js';

function containerPrefix() {
  return String(projectState.bundle?.containerPrefix || '').trim();
}

export function containerName(serviceKey) {
  const prefix = containerPrefix();
  return prefix ? `${prefix}-${serviceKey}` : serviceKey;
}

/** 容器清单来自当前项目交付包声明的服务，客户端不再内置项目拓扑。 */
export function clusterServices() {
  const services = Array.isArray(projectState.bundle?.services) ? projectState.bundle.services : [];
  return services.map((service) => ({
    container: containerName(service.key),
    serviceKey: service.key,
    label: service.label || service.key,
    port: service.port ?? null,
    protocol: service.protocol || 'TCP',
    description: service.description || ''
  }));
}

export function serviceOptions() {
  return clusterServices().map((service) => ({
    name: service.container,
    label: service.port ? `${service.label} :${service.port}` : service.label
  }));
}
