import { z } from 'zod';
import { Bundle } from '../bundle.mjs';
import { appVersion } from '../config.mjs';
import { DeployError } from '../errors.mjs';
import {
  loadProjectStore,
  mutateProjectStore,
  nextProjectId,
  requireProject,
  resolveManagedHost,
  validateInstallDir,
  validateProjectName
} from '../projects.mjs';
import { loadArtifactSources, requireArtifactSource } from '../artifact-sources.mjs';
import { readOnly, mutating, destructive } from './shared.mjs';

const projectSelector = z.string().min(1).describe('项目 ID 或名称');
const hostSelector = z.string().min(1).describe('主机 ID、名称或地址，需为已保存的部署主机');

function projectPayload(store, path) {
  return {
    path,
    activeProjectId: store.activeProjectId,
    activeProject: store.projects.find((project) => project.id === store.activeProjectId) || store.projects[0] || null,
    projects: store.projects
  };
}

async function resolveSourceId(selector) {
  if (selector === undefined) return undefined;
  const value = String(selector ?? '').trim();
  if (!value) return '';
  const sources = await loadArtifactSources();
  return requireArtifactSource(sources, value).id;
}

export function registerProjectsTools(register) {
  register('deploy_project_list', {
    title: '查看项目',
    description: '列出已保存的项目、当前项目，以及每个项目绑定的主机、部署目录与交付包。',
    inputSchema: {},
    annotations: readOnly
  }, async () => {
    const store = await loadProjectStore();
    return projectPayload(store, store.path);
  });

  register('deploy_project_create', {
    title: '新建项目',
    description: '创建项目并绑定目标主机、部署目录与可选制品源。项目只保存定位信息。',
    inputSchema: {
      name: z.string().min(1).describe('项目名称，最多 60 个字符'),
      host: hostSelector,
      installDir: z.string().min(1).describe('目标机上的绝对部署目录'),
      source: z.string().optional().describe('制品源 ID 或名称；留空表示暂不绑定'),
      activate: z.boolean().optional().default(false).describe('创建后设为当前项目；当前没有项目时会自动设为当前项目')
    },
    annotations: mutating
  }, async (args) => {
    const name = validateProjectName(args.name);
    const installDir = validateInstallDir(args.installDir);
    const host = await resolveManagedHost(args.host);
    const sourceId = (await resolveSourceId(args.source)) || '';
    let created = null;
    const { store, path } = await mutateProjectStore((draft) => {
      const project = {
        id: nextProjectId(draft),
        name,
        product: '',
        hostId: host.id,
        installDir,
        sourceId,
        lastVersion: '',
        lastBundleDir: ''
      };
      draft.schemaVersion = draft.schemaVersion || 1;
      draft.projects.push(project);
      if (!draft.activeProjectId.trim() || args.activate === true) draft.activeProjectId = project.id;
      created = project;
    });
    return { ...projectPayload(store, path), project: created };
  });

  register('deploy_project_update', {
    title: '编辑项目',
    description: '调整项目的名称、主机、部署目录或制品源；只修改传入的字段。',
    inputSchema: {
      project: projectSelector,
      name: z.string().optional().describe('新的项目名称'),
      host: hostSelector.optional().describe('新的目标主机；留空保持不变'),
      installDir: z.string().optional().describe('新的绝对部署目录；留空保持不变'),
      source: z.string().optional().describe('新的制品源 ID 或名称；传空字符串表示清除绑定')
    },
    annotations: mutating
  }, async (args) => {
    if (args.name === undefined && args.host === undefined && args.installDir === undefined && args.source === undefined) {
      throw new DeployError('请至少传入一个要修改的字段', { code: 'PROJECT_UPDATE_EMPTY' });
    }
    const host = args.host === undefined ? null : await resolveManagedHost(args.host);
    const sourceId = await resolveSourceId(args.source);
    let updated = null;
    const { store, path } = await mutateProjectStore((draft) => {
      const project = requireProject(draft, args.project);
      if (args.name !== undefined) project.name = validateProjectName(args.name);
      if (host) project.hostId = host.id;
      if (args.installDir !== undefined) project.installDir = validateInstallDir(args.installDir);
      if (sourceId !== undefined) project.sourceId = sourceId;
      updated = project;
    });
    return { ...projectPayload(store, path), project: updated };
  });

  register('deploy_project_remove', {
    title: '删除项目',
    description: '删除项目记录；已下载的交付包与目标机上的文件不会被删除。',
    inputSchema: { project: projectSelector, confirm: z.boolean().optional() },
    annotations: destructive
  }, async (args) => {
    if (args.confirm !== true) {
      throw new DeployError('删除项目会移除项目配置，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    }
    let removed = null;
    const { store, path } = await mutateProjectStore((draft) => {
      const project = requireProject(draft, args.project);
      removed = { ...project };
      draft.projects = draft.projects.filter((item) => item.id !== project.id);
      if (draft.activeProjectId === project.id) draft.activeProjectId = draft.projects[0]?.id || '';
    });
    return { ...projectPayload(store, path), removed };
  });

  register('deploy_project_activate', {
    title: '切换当前项目',
    description: '把指定项目设为后续部署与运维工具使用的当前项目。',
    inputSchema: { project: projectSelector },
    annotations: mutating
  }, async (args) => {
    let activated = null;
    const { store, path } = await mutateProjectStore((draft) => {
      const project = requireProject(draft, args.project);
      draft.activeProjectId = project.id;
      activated = project;
    });
    return { ...projectPayload(store, path), project: activated };
  });

  register('deploy_project_bind_bundle', {
    title: '绑定交付包',
    description: '把本机已下载的交付包绑定到项目，并校验交付包版本与产品是否匹配。',
    inputSchema: {
      project: projectSelector,
      bundleDir: z.string().min(1).describe('交付包目录，需包含 bundle.json'),
      version: z.string().optional().describe('期望的交付包版本；填写后会与包内版本核对')
    },
    annotations: mutating
  }, async (args) => {
    const bundle = await Bundle.load(args.bundleDir, { appVersion: await appVersion() });
    const expected = String(args.version ?? '').trim();
    if (expected && bundle.version() !== expected) {
      throw new DeployError(`交付包版本为 ${bundle.version()}，与所选版本 ${expected} 不一致，请重新下载制品`, {
        code: 'ARTIFACT_VERSION_MISMATCH'
      });
    }
    let bound = null;
    const { store, path } = await mutateProjectStore((draft) => {
      const project = requireProject(draft, args.project);
      if (project.product.trim() && project.product.trim() !== bundle.product()) {
        throw new DeployError(`该交付包属于「${bundle.product()}」，当前项目已绑定「${project.product}」，请选择匹配的制品`, {
          code: 'PRODUCT_MISMATCH'
        });
      }
      project.product = bundle.product();
      project.lastVersion = bundle.version();
      project.lastBundleDir = bundle.root;
      bound = project;
    });
    return {
      ...projectPayload(store, path),
      project: bound,
      bundle: {
        dir: bundle.root,
        product: bundle.product(),
        displayName: bundle.metadata.displayName,
        version: bundle.version(),
        buildNumber: bundle.metadata.buildNumber,
        commitShort: bundle.metadata.commitShort,
        branch: bundle.metadata.branch,
        builtAt: bundle.metadata.builtAt
      }
    };
  });
}
