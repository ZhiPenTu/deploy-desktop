import fs from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';
import { DeployError } from './errors.mjs';
import { compareVersions, isObject } from './util.mjs';

export const API_VERSION = 'deploy.desktop/bundle/v2';
export const KIND = 'DeploymentBundle';
export const IMAGE_COMPONENT = 'image';

const IGNORED_FILE = '.DS_Store';

const FIELDS = {
  bundle: ['apiVersion', 'kind', 'metadata', 'runtime', 'components', 'artifacts', 'images', 'plan', 'files'],
  metadata: ['product', 'displayName', 'version', 'buildNumber', 'commit', 'commitShort', 'branch', 'builtAt', 'minToolVersion'],
  runtime: ['type', 'compose', 'variant', 'containerPrefix', 'services', 'database'],
  service: ['key', 'label', 'port', 'protocol', 'description', 'container'],
  database: ['serviceKey', 'name', 'client', 'port'],
  component: ['key', 'label', 'artifacts', 'services'],
  artifact: ['key', 'label', 'source', 'root', 'path', 'type'],
  image: ['name', 'ref', 'role', 'required', 'bundleFile'],
  plan: ['steps'],
  step: ['key', 'title', 'hint'],
  file: ['path', 'size', 'sha256']
};

function fail(message, code = 'BUNDLE_INVALID', details) {
  return new DeployError(message, { code, details });
}

function assertFields(value, allowed, label) {
  if (!isObject(value)) throw fail(`${label} 必须是 JSON 对象`);
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) throw fail(`${label} 包含未知字段：${key}`, 'BUNDLE_UNKNOWN_FIELD', { field: key });
  }
}

function text(value, label, { allowEmpty = true } = {}) {
  if (value === undefined || value === null) {
    if (allowEmpty) return '';
    throw fail(`${label} 不能为空`);
  }
  if (typeof value !== 'string') throw fail(`${label} 必须是字符串`);
  const trimmed = value.trim();
  if (!allowEmpty && !trimmed) throw fail(`${label} 不能为空`);
  return trimmed;
}

function list(value, label) {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) throw fail(`${label} 必须是数组`);
  return value;
}

export function safeRelative(value, label) {
  const raw = typeof value === 'string' ? value.trim() : '';
  if (!raw) throw fail(`${label}为空，请重新构建制品`);
  if (raw.includes('\\')) throw fail(`${label}必须使用 / 分隔：${raw}`);
  if (path.isAbsolute(raw) || /^[A-Za-z]:/.test(raw)) throw fail(`${label}不能是绝对路径：${raw}`);
  const parts = raw.split('/');
  if (parts.some((part) => part === '' || part === '.' || part === '..')) {
    throw fail(`${label}包含不合法路径：${raw}`);
  }
  return parts.join('/');
}

function normalizeService(value, index) {
  assertFields(value, FIELDS.service, `runtime.services[${index}]`);
  const key = text(value.key, `runtime.services[${index}].key`, { allowEmpty: false });
  const port = value.port === undefined || value.port === null ? null : Number(value.port);
  if (port !== null && (!Number.isInteger(port) || port < 1 || port > 65535)) {
    throw fail(`服务 ${key} 的端口必须是 1-65535 的整数`);
  }
  const protocol = text(value.protocol, `runtime.services[${index}].protocol`) || 'TCP';
  if (!['TCP', 'UDP'].includes(protocol.toUpperCase())) {
    throw fail(`服务 ${key} 的协议必须是 TCP 或 UDP`);
  }
  return {
    serviceKey: key,
    label: text(value.label, `runtime.services[${index}].label`, { allowEmpty: false }),
    port,
    protocol: protocol.toUpperCase(),
    description: text(value.description, `runtime.services[${index}].description`),
    container: text(value.container, `runtime.services[${index}].container`)
  };
}

function normalizeArtifact(value, index) {
  assertFields(value, FIELDS.artifact, `artifacts[${index}]`);
  const key = text(value.key, `artifacts[${index}].key`, { allowEmpty: false });
  const root = text(value.root, `artifacts[${index}].root`, { allowEmpty: false });
  if (!['install', 'deploy'].includes(root)) throw fail(`产物 ${key} 的 root 必须是 install 或 deploy`);
  const kind = text(value.type, `artifacts[${index}].type`, { allowEmpty: false });
  if (!['file', 'dir'].includes(kind)) throw fail(`产物 ${key} 的 type 必须是 file 或 dir`);
  return {
    key,
    label: text(value.label, `artifacts[${index}].label`),
    source: safeRelative(value.source, '产物来源路径'),
    root,
    path: safeRelative(value.path, '产物部署路径'),
    kind
  };
}

export function validateDescriptor(raw) {
  assertFields(raw, FIELDS.bundle, 'bundle.json');
  const apiVersion = text(raw.apiVersion, 'apiVersion', { allowEmpty: false });
  if (apiVersion !== API_VERSION) {
    throw fail('该交付包使用旧格式，当前版本已不再支持，请使用新版流水线重新构建并上传制品', 'BUNDLE_VERSION_UNSUPPORTED');
  }
  const kind = text(raw.kind, 'kind', { allowEmpty: false });
  if (kind !== KIND) throw fail('交付包类型不正确，请使用新版流水线重新构建并上传制品');

  assertFields(raw.metadata, FIELDS.metadata, 'metadata');
  const metadata = {
    product: text(raw.metadata.product, 'metadata.product', { allowEmpty: false }),
    displayName: text(raw.metadata.displayName, 'metadata.displayName'),
    version: text(raw.metadata.version, 'metadata.version', { allowEmpty: false }),
    buildNumber: text(raw.metadata.buildNumber, 'metadata.buildNumber'),
    commit: text(raw.metadata.commit, 'metadata.commit'),
    commitShort: text(raw.metadata.commitShort, 'metadata.commitShort'),
    branch: text(raw.metadata.branch, 'metadata.branch'),
    builtAt: text(raw.metadata.builtAt, 'metadata.builtAt'),
    minToolVersion: text(raw.metadata.minToolVersion, 'metadata.minToolVersion')
  };

  assertFields(raw.runtime, FIELDS.runtime, 'runtime');
  const runtimeType = text(raw.runtime.type, 'runtime.type', { allowEmpty: false });
  if (runtimeType.toLowerCase() !== 'compose') {
    throw fail(`暂不支持「${runtimeType}」类型的部署方式，请改用 Compose 交付包`, 'RUNTIME_UNSUPPORTED');
  }
  const compose = safeRelative(raw.runtime.compose, 'Compose 清单路径');
  const prefix = text(raw.runtime.containerPrefix, 'runtime.containerPrefix');
  if (/[\r\n\0]/.test(prefix)) throw fail('交付包声明的容器前缀无效');
  const services = list(raw.runtime.services, 'runtime.services').map(normalizeService);
  const serviceKeys = new Set();
  for (const service of services) {
    if (serviceKeys.has(service.serviceKey)) throw fail(`交付包重复声明服务 ${service.serviceKey}`);
    serviceKeys.add(service.serviceKey);
  }
  let database = null;
  if (raw.runtime.database !== undefined && raw.runtime.database !== null) {
    assertFields(raw.runtime.database, FIELDS.database, 'runtime.database');
    const port = Number(raw.runtime.database.port);
    if (!Number.isInteger(port) || port < 1 || port > 65535) throw fail('交付包的数据库端口无效');
    database = {
      serviceKey: text(raw.runtime.database.serviceKey, 'runtime.database.serviceKey', { allowEmpty: false }),
      name: text(raw.runtime.database.name, 'runtime.database.name', { allowEmpty: false }),
      client: text(raw.runtime.database.client, 'runtime.database.client', { allowEmpty: false }),
      port
    };
    if (!serviceKeys.has(database.serviceKey)) {
      throw fail(`交付包的数据库服务 ${database.serviceKey} 未在服务清单中声明`);
    }
  }
  const runtime = {
    type: 'compose',
    compose,
    variant: raw.runtime.variant ? safeRelative(raw.runtime.variant, 'Compose 变体路径') : '',
    containerPrefix: prefix,
    services,
    database
  };

  const artifacts = list(raw.artifacts, 'artifacts').map(normalizeArtifact);
  const artifactKeys = new Set();
  const targets = new Set();
  for (const artifact of artifacts) {
    if (artifactKeys.has(artifact.key)) throw fail(`交付包重复声明产物 ${artifact.key}`);
    artifactKeys.add(artifact.key);
    if (artifact.source === 'bundle.json') throw fail('产物来源不能是 bundle.json');
    const target = `${artifact.root}\0${artifact.path}`;
    if (targets.has(target)) throw fail(`交付包重复声明部署位置 ${artifact.path}`);
    targets.add(target);
  }
  if (!artifacts.some((artifact) => artifact.source === compose)) {
    throw fail(`交付包未将 Compose 清单 ${compose} 声明为产物`);
  }

  const components = list(raw.components, 'components').map((value, index) => {
    assertFields(value, FIELDS.component, `components[${index}]`);
    const key = text(value.key, `components[${index}].key`, { allowEmpty: false });
    if (key === IMAGE_COMPONENT) throw fail('交付包组件标识不能使用 image');
    return {
      key,
      label: text(value.label, `components[${index}].label`, { allowEmpty: false }),
      artifacts: list(value.artifacts, `components[${index}].artifacts`).map((item, position) => text(item, `components[${index}].artifacts[${position}]`, { allowEmpty: false })),
      services: list(value.services, `components[${index}].services`).map((item, position) => text(item, `components[${index}].services[${position}]`, { allowEmpty: false }))
    };
  });
  const componentKeys = new Set();
  for (const component of components) {
    if (componentKeys.has(component.key)) throw fail(`交付包重复声明组件 ${component.key}`);
    componentKeys.add(component.key);
    for (const key of component.artifacts) {
      if (!artifactKeys.has(key)) throw fail(`组件 ${component.key} 引用了未声明的产物 ${key}`);
    }
    for (const key of component.services) {
      if (!serviceKeys.has(key)) throw fail(`组件 ${component.key} 引用了未声明的服务 ${key}`);
    }
  }

  const images = list(raw.images, 'images').map((value, index) => {
    assertFields(value, FIELDS.image, `images[${index}]`);
    const name = text(value.name, `images[${index}].name`, { allowEmpty: false });
    const reference = text(value.ref, `images[${index}].ref`, { allowEmpty: false });
    if (/[\r\n\0\s]/.test(reference)) throw fail(`交付包镜像 ${name} 的引用无效`);
    const role = text(value.role, `images[${index}].role`, { allowEmpty: false });
    if (!['app', 'base'].includes(role)) throw fail(`镜像 ${name} 的 role 必须是 app 或 base`);
    return {
      name,
      imageRef: reference,
      role,
      required: Boolean(value.required),
      bundleFile: value.bundleFile ? safeRelative(value.bundleFile, '镜像包路径') : ''
    };
  });

  assertFields(raw.plan === undefined ? {} : raw.plan, FIELDS.plan, 'plan');
  const steps = list(raw.plan?.steps, 'plan.steps').map((value, index) => {
    assertFields(value, FIELDS.step, `plan.steps[${index}]`);
    return {
      key: text(value.key, `plan.steps[${index}].key`, { allowEmpty: false }),
      title: text(value.title, `plan.steps[${index}].title`),
      hint: text(value.hint, `plan.steps[${index}].hint`)
    };
  });

  const files = list(raw.files, 'files').map((value, index) => {
    assertFields(value, FIELDS.file, `files[${index}]`);
    const filePath = safeRelative(value.path, '交付包文件路径');
    const size = Number(value.size);
    if (!Number.isInteger(size) || size < 0) throw fail(`交付包文件 ${filePath} 的大小无效`);
    const sha256 = text(value.sha256, `files[${index}].sha256`, { allowEmpty: false }).toLowerCase();
    if (!/^[0-9a-f]{64}$/.test(sha256)) throw fail(`交付包文件 ${filePath} 的摘要无效`);
    return { path: filePath, size, sha256 };
  });

  const filePaths = new Set(files.map((file) => file.path));
  if (filePaths.size !== files.length) throw fail('交付包重复声明文件');
  if (!filePaths.has(compose)) throw fail(`交付包未声明 Compose 清单 ${compose}`);

  const ownership = new Map();
  for (const artifact of artifacts) {
    const owned = artifact.kind === 'file'
      ? [artifact.source]
      : [...filePaths].filter((item) => item.startsWith(`${artifact.source}/`));
    if (owned.length === 0) throw fail(`产物 ${artifact.key} 的目录 ${artifact.source} 在交付包中为空`);
    if (artifact.kind === 'file' && !filePaths.has(artifact.source)) {
      throw fail(`产物 ${artifact.key} 的来源 ${artifact.source} 不在交付包文件清单中`);
    }
    for (const item of owned) {
      if (ownership.has(item)) throw fail(`交付包文件 ${item} 同时归属于 ${ownership.get(item)} 与 ${artifact.key}`);
      ownership.set(item, artifact.key);
    }
  }
  for (const image of images) {
    if (!image.bundleFile) continue;
    if (!filePaths.has(image.bundleFile)) throw fail(`镜像包 ${image.bundleFile} 不在交付包文件清单中`);
    if (ownership.has(image.bundleFile)) {
      throw fail(`交付包文件 ${image.bundleFile} 同时归属于 ${ownership.get(image.bundleFile)} 与镜像 ${image.name}`);
    }
    ownership.set(image.bundleFile, `image:${image.name}`);
  }
  for (const file of filePaths) {
    if (!ownership.has(file)) throw fail(`交付包文件 ${file} 未归属于任何产物或镜像`);
  }

  return {
    apiVersion,
    kind,
    metadata,
    runtime,
    components,
    artifacts,
    images,
    plan: { steps },
    files
  };
}

async function collectFiles(root, dir, out) {
  const entries = await fs.readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isSymbolicLink()) {
      throw fail(`交付包包含符号链接：${path.relative(root, full)}`, 'BUNDLE_SYMLINK');
    }
    if (entry.isDirectory()) {
      await collectFiles(root, full, out);
      continue;
    }
    if (entry.name === IGNORED_FILE) continue;
    const relative = path.relative(root, full).split(path.sep).join('/');
    if (relative === 'bundle.json') continue;
    const stat = await fs.stat(full);
    out.set(relative, stat.size);
  }
}

export class Bundle {
  constructor(root, descriptor) {
    this.root = root;
    this.descriptor = descriptor;
  }

  static async load(root, options = {}) {
    const resolved = path.resolve(root);
    const manifest = path.join(resolved, 'bundle.json');
    let content;
    try {
      content = await fs.readFile(manifest, 'utf8');
    } catch {
      throw fail('交付包缺少 bundle.json，请使用新版流水线重新构建并上传制品', 'BUNDLE_NOT_FOUND', { path: resolved });
    }
    let raw;
    try {
      raw = JSON.parse(content);
    } catch (error) {
      throw fail(`bundle.json 不是有效 JSON：${error.message}`, 'BUNDLE_INVALID', { path: manifest });
    }
    const bundle = new Bundle(resolved, validateDescriptor(raw));
    if (options.appVersion) bundle.ensureCompatible(options.appVersion);
    if (options.verify) await bundle.verifyPayload();
    return bundle;
  }

  get metadata() {
    return this.descriptor.metadata;
  }

  product() {
    return this.descriptor.metadata.product;
  }

  version() {
    return this.descriptor.metadata.version;
  }

  deploySourceRel() {
    return path.posix.dirname(this.descriptor.runtime.compose) === '.' ? '' : path.posix.dirname(this.descriptor.runtime.compose);
  }

  deploySourceDir() {
    const rel = this.deploySourceRel();
    return rel ? path.join(this.root, rel) : this.root;
  }

  component(key) {
    return this.descriptor.components.find((item) => item.key === key) || null;
  }

  artifact(key) {
    return this.descriptor.artifacts.find((item) => item.key === key) || null;
  }

  artifactForSource(source) {
    const target = String(source || '').trim();
    return this.descriptor.artifacts.find((item) => item.source === target) || null;
  }

  service(key) {
    return this.descriptor.runtime.services.find((item) => item.serviceKey === key) || null;
  }

  selectComponents(keys) {
    const selected = [];
    for (const raw of keys) {
      const key = String(raw || '').trim();
      if (!key || key === IMAGE_COMPONENT) continue;
      const component = this.component(key);
      if (!component) throw fail(`交付包未声明组件 ${key}，请重新选择制品`, 'COMPONENT_UNKNOWN');
      if (!selected.some((item) => item.key === key)) selected.push(component);
    }
    return selected;
  }

  artifactsFor(components) {
    const result = [];
    for (const component of components) {
      for (const key of component.artifacts) {
        const artifact = this.artifact(key);
        if (artifact && !result.some((item) => item.key === artifact.key)) result.push(artifact);
      }
    }
    return result;
  }

  serviceKeysForComponent(key) {
    if (key === 'config') return this.descriptor.runtime.services.map((item) => item.serviceKey);
    return [...(this.component(key)?.services || [])];
  }

  resolveArtifacts(components, installDir, deployDir) {
    return this.artifactsFor(components).map((artifact) => {
      const base = artifact.root === 'install' ? installDir : deployDir;
      const target = path.resolve(base, artifact.path);
      const relative = path.relative(path.resolve(base), target);
      if (relative.startsWith('..') || path.isAbsolute(relative)) {
        throw fail(`产物 ${artifact.key} 的部署路径超出部署目录`, 'ARTIFACT_OUTSIDE_ROOT');
      }
      return {
        key: artifact.key,
        label: artifact.label,
        root: artifact.root,
        path: artifact.path,
        kind: artifact.kind,
        source: path.join(this.root, artifact.source),
        target
      };
    });
  }

  containerName(serviceKey) {
    const service = this.service(serviceKey);
    if (service?.container) return service.container;
    return this.descriptor.runtime.containerPrefix
      ? `${this.descriptor.runtime.containerPrefix}-${serviceKey}`
      : serviceKey;
  }

  imageFiles() {
    return this.descriptor.images
      .filter((image) => image.bundleFile)
      .map((image) => path.join(this.root, image.bundleFile));
  }

  hasBundleImages() {
    return this.descriptor.images.some((image) => Boolean(image.bundleFile));
  }

  ensureCompatible(toolVersion) {
    const required = this.descriptor.metadata.minToolVersion;
    if (!required) return;
    if (compareVersions(toolVersion, required) < 0) {
      throw fail(`该交付包需要部署舱 ${required} 或更高版本，当前为 ${toolVersion}，请先升级客户端`, 'TOOL_VERSION_TOO_LOW');
    }
  }

  async verifyPayload() {
    const actual = new Map();
    await collectFiles(this.root, this.root, actual);
    const declared = new Map(this.descriptor.files.map((file) => [file.path, file]));
    for (const [file, size] of actual) {
      const spec = declared.get(file);
      if (!spec) throw fail(`交付包包含未声明的文件：${file}`, 'BUNDLE_EXTRA_FILE');
      if (spec.size !== size) throw fail(`交付包文件大小与清单不一致：${file}`, 'BUNDLE_SIZE_MISMATCH');
    }
    for (const spec of this.descriptor.files) {
      if (!actual.has(spec.path)) throw fail(`交付包缺少文件：${spec.path}`, 'BUNDLE_FILE_MISSING');
      const bytes = await fs.readFile(path.join(this.root, spec.path));
      const digest = crypto.createHash('sha256').update(bytes).digest('hex');
      if (digest !== spec.sha256) throw fail(`交付包文件校验失败：${spec.path}`, 'BUNDLE_DIGEST_MISMATCH');
    }
  }
}

export function normalizeComponents(bundle, input = []) {
  const result = [];
  for (const raw of input) {
    const key = String(raw || '').trim();
    if (!key) continue;
    if (key === IMAGE_COMPONENT) {
      if (!bundle.hasBundleImages()) throw fail('该交付包未包含离线镜像，无需选择镜像更新', 'COMPONENT_IMAGE_UNAVAILABLE');
      if (!result.includes(key)) result.push(key);
      continue;
    }
    if (!bundle.component(key)) throw fail(`交付包未声明组件 ${key}，请重新选择制品`, 'COMPONENT_UNKNOWN');
    if (!result.includes(key)) result.push(key);
  }
  if (result.length === 0) throw fail('请至少选择一个更新组件', 'COMPONENT_REQUIRED');
  return result;
}

export function defaultComponents(bundle) {
  const keys = bundle.descriptor.components.map((component) => component.key);
  return bundle.hasBundleImages() ? [...keys, IMAGE_COMPONENT] : keys;
}
