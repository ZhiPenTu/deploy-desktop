import path from 'node:path';
import { DeployError } from './errors.mjs';
import { appVersion } from './config.mjs';
import { Bundle } from './bundle.mjs';
import { activeProject } from './projects.mjs';
import { expandHome, isObject } from './util.mjs';

export { appVersion };

function requireText(value, field) {
  if (typeof value !== 'string' || !value.trim()) throw new DeployError(`${field} 不能为空`);
  if (/[\r\n]/.test(value)) throw new DeployError(`${field} 不能包含换行`);
  return value.trim();
}

/** 由交付包与项目推导出的运行时视图；项目中只保存定位信息。 */
export function buildProfile(project, bundle, options = {}) {
  const installDir = path.resolve(expandHome(options.installDir || project.installDir));
  const deployDir = path.resolve(installDir, bundle.deploySourceRel());
  const composeArtifact = bundle.artifactForSource(bundle.descriptor.runtime.compose);
  const composeFile = path.join(deployDir, composeArtifact.path);
  let variantFile = '';
  const variant = bundle.descriptor.runtime.variant;
  if (variant) {
    const variantArtifact = bundle.artifactForSource(variant);
    variantFile = path.join(deployDir, variantArtifact ? variantArtifact.path : path.posix.basename(variant));
  }
  const services = bundle.descriptor.runtime.services.map((service) => ({ ...service }));
  return {
    projectId: project.id,
    name: project.name,
    product: bundle.product(),
    displayName: bundle.metadata.displayName,
    version: bundle.version(),
    containerPrefix: bundle.descriptor.runtime.containerPrefix,
    bundleDir: bundle.root,
    installDir,
    deployDir,
    deploySourceRel: bundle.deploySourceRel(),
    compose: {
      source: bundle.descriptor.runtime.compose,
      variant,
      file: composeFile,
      variantFile,
      fileRelative: path.relative(deployDir, composeFile).split(path.sep).join('/'),
      variantRelative: variantFile ? path.relative(deployDir, variantFile).split(path.sep).join('/') : ''
    },
    components: bundle.descriptor.components.map((component) => ({
      key: component.key,
      label: component.label,
      artifacts: [...component.artifacts],
      services: [...component.services]
    })),
    artifacts: bundle.descriptor.artifacts.map((artifact) => ({ ...artifact })),
    services,
    containers: services,
    database: bundle.descriptor.runtime.database ? { ...bundle.descriptor.runtime.database } : null,
    images: bundle.descriptor.images.map((image) => ({
      name: image.name,
      imageRef: image.imageRef,
      role: image.role,
      required: image.required,
      path: image.bundleFile,
      file: image.bundleFile ? path.posix.basename(image.bundleFile) : ''
    })),
    steps: bundle.descriptor.plan.steps.map((step) => ({ ...step }))
  };
}

export function validateProfile(profile) {
  if (!isObject(profile)) throw new DeployError('部署配置必须是对象');
  const result = {
    ...profile,
    name: requireText(profile.name, 'name'),
    product: requireText(profile.product, 'product'),
    containerPrefix: requireText(profile.containerPrefix, 'containerPrefix'),
    bundleDir: requireText(profile.bundleDir, 'bundleDir'),
    installDir: requireText(profile.installDir, 'installDir'),
    deployDir: requireText(profile.deployDir, 'deployDir'),
    containers: Array.isArray(profile.containers) ? profile.containers.map((item) => ({ ...item })) : [],
    components: Array.isArray(profile.components) ? profile.components.map((item) => ({ ...item })) : [],
    images: Array.isArray(profile.images) ? profile.images.map((item) => ({ ...item })) : []
  };
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(result.containerPrefix)) {
    throw new DeployError('部署配置中的容器前缀只能包含字母、数字、点、下划线和短横线');
  }
  if (!path.isAbsolute(result.installDir)) throw new DeployError('部署目录必须是绝对路径');
  if (!path.isAbsolute(result.deployDir)) throw new DeployError('部署清单目录必须是绝对路径');
  const keys = new Set();
  for (const container of result.containers) {
    const key = requireText(container.serviceKey, 'serviceKey');
    if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(key)) throw new DeployError(`服务键无效: ${key}`);
    if (keys.has(key)) throw new DeployError(`服务键重复: ${key}`);
    keys.add(key);
  }
  if (result.database && !keys.has(result.database.serviceKey)) {
    throw new DeployError(`数据库服务未出现在服务清单中: ${result.database.serviceKey}`);
  }
  return result;
}

async function projectAndBundle(bundleDir) {
  const { project } = await activeProject();
  const root = path.resolve(expandHome(bundleDir || project.lastBundleDir));
  if (!bundleDir && !project.lastBundleDir) {
    throw new DeployError(`项目「${project.name}」尚未绑定交付包`, {
      code: 'BUNDLE_NOT_BOUND',
      hint: '请先在制品中心选择并绑定一个交付包。'
    });
  }
  const bundle = await Bundle.load(root, { appVersion: await appVersion() });
  if (project.product && project.product !== bundle.product()) {
    throw new DeployError(`交付包属于「${bundle.product()}」，项目已绑定「${project.product}」`, {
      code: 'PRODUCT_MISMATCH'
    });
  }
  return { project, bundle };
}

export async function loadProfile(options = {}) {
  const { project, bundle } = await projectAndBundle(options.bundleDir);
  const profile = validateProfile(buildProfile(project, bundle, options));
  return { profile, project, bundle, source: profile.bundleDir };
}

export function containerName(profile, serviceKey) {
  const service = profile.containers.find((item) => item.serviceKey === serviceKey);
  if (service?.container) return service.container;
  return profile.containerPrefix ? `${profile.containerPrefix}-${serviceKey}` : serviceKey;
}

export function resolveContainer(profile, input) {
  const value = String(input || '').trim();
  if (!value) throw new DeployError('请填写服务名或容器名');
  if (profile.containers.some((item) => item.container === value)) return value;
  if (profile.containerPrefix && value.startsWith(`${profile.containerPrefix}-`)) return value;
  const service = profile.containers.find((item) => item.serviceKey === value);
  if (!service) {
    throw new DeployError(`交付包中不存在服务: ${value}`, {
      hint: `可用服务: ${profile.containers.map((item) => item.serviceKey).join(', ')}`
    });
  }
  return containerName(profile, value);
}

export function resolveServiceKey(profile, input) {
  const value = String(input || '').trim();
  if (!value) return '';
  if (profile.containers.some((item) => item.serviceKey === value)) return value;
  const match = profile.containers.find((item) => containerName(profile, item.serviceKey) === value);
  if (match) return match.serviceKey;
  throw new DeployError(`交付包中不存在服务: ${value}`, {
    hint: `可用服务: ${profile.containers.map((item) => item.serviceKey).join(', ')}`
  });
}

export function databaseContainer(profile) {
  return containerName(profile, profile.database.serviceKey);
}

export function serviceKeysForComponent(profile, component) {
  if (component === 'config') return profile.containers.map((item) => item.serviceKey);
  const declared = profile.components.find((item) => item.key === component);
  return declared ? [...declared.services] : [];
}

export async function projectBundleInfo() {
  const { project, bundle } = await projectAndBundle();
  return { project, bundle, profile: buildProfile(project, bundle) };
}
