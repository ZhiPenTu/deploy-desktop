import profile from '../../profile.json';

export const defaultCluster = {
  name: profile.name,
  services: profile.containers.map((container) => ({
    container: `${profile.containerPrefix}-${container.serviceKey}`,
    serviceKey: container.serviceKey,
    label: container.label,
    port: container.port ?? null,
    protocol: container.protocol,
    description: container.description
  }))
};

export const defaultContainer = defaultCluster.services[0].container;

export function serviceOptions() {
  return defaultCluster.services.map((service) => ({
    name: service.container,
    label: service.port ? `${service.label} :${service.port}` : service.label
  }));
}
