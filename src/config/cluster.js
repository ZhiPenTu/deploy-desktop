import { profileState } from './profile.js';

function containerName(serviceKey) {
  return profileState.containerPrefix ? `${profileState.containerPrefix}-${serviceKey}` : serviceKey;
}

function clusterServices() {
  return profileState.containers.map((container) => ({
    container: containerName(container.serviceKey),
    serviceKey: container.serviceKey,
    label: container.label,
    port: container.port ?? null,
    protocol: container.protocol,
    description: container.description
  }));
}

export const defaultCluster = {
  get name() {
    return profileState.name;
  },
  get services() {
    return clusterServices();
  }
};

export function getDefaultContainer() {
  const services = clusterServices();
  return services.length ? services[0].container : '';
}

export function serviceOptions() {
  return clusterServices().map((service) => ({
    name: service.container,
    label: service.port ? `${service.label} :${service.port}` : service.label
  }));
}
