import { reactive } from 'vue';
import {
  activateProjectBundle,
  getActiveProjectContext,
  listProjects,
  setActiveProject
} from '../api/tauri.js';
import { errorText } from '../shared/errors.js';

/**
 * 当前项目状态：项目只记录部署目标（主机、部署目录、制品源），
 * 组件、服务与数据库全部来自当前项目绑定的交付包。
 */
export const projectState = reactive({
  projects: [],
  activeProjectId: '',
  project: null,
  bundle: null,
  deployDir: '',
  loading: false,
  loaded: false,
  error: ''
});

function applyStore(store) {
  const projects = Array.isArray(store?.projects) ? store.projects : [];
  projectState.projects = projects;
  projectState.activeProjectId = store?.activeProjectId || projects[0]?.id || '';
}

function applyContext(context) {
  projectState.project = context?.project || null;
  projectState.bundle = context?.bundle || null;
  projectState.deployDir = context?.deployDir || '';
  projectState.activeProjectId = context?.project?.id || projectState.activeProjectId;
}

export function hasProject() {
  return Boolean(projectState.project?.id);
}

export function hasBundle() {
  return Boolean(projectState.bundle);
}

export async function refreshProjectContext() {
  projectState.loading = true;
  projectState.error = '';
  try {
    const [store, context] = await Promise.all([
      listProjects(),
      getActiveProjectContext()
    ]);
    applyStore(store);
    applyContext(context);
    projectState.loaded = true;
    return context;
  } catch (error) {
    projectState.error = errorText(error);
    throw error;
  } finally {
    projectState.loading = false;
  }
}

export async function refreshProjects() {
  applyStore(await listProjects());
}

export async function initProjects() {
  try {
    return await refreshProjectContext();
  } catch (error) {
    console.warn('读取项目配置失败，可在「项目管理」中重新创建项目', error);
    projectState.loaded = true;
    return null;
  }
}

export async function switchProject(id) {
  const store = await setActiveProject(id);
  applyStore(store);
  return await refreshProjectContext();
}

/** 将当前项目绑定到所选交付包；项目尚未创建时给出可操作提示。 */
export async function bindProjectBundle(bundleDir, version) {
  const project = projectState.project;
  if (!project?.id) {
    throw new Error('请先在「项目管理」中新建项目并选择目标主机');
  }
  await activateProjectBundle(project.id, String(bundleDir || '').trim(), String(version || '').trim());
  return await refreshProjectContext();
}
