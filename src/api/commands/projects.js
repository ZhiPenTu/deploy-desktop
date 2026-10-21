import { invokeDesktop, isDesktopApp } from '../desktop.js';

export async function listProjects() {
  if (!isDesktopApp()) return { schemaVersion: 1, activeProjectId: '', projects: [] };
  return await invokeDesktop('list_projects');
}

export async function createProject(input) {
  return await invokeDesktop('create_project', { input });
}

export async function updateProject(input) {
  return await invokeDesktop('update_project', { input });
}

export async function deleteProject(id) {
  return await invokeDesktop('delete_project', { id });
}

export async function setActiveProject(id) {
  return await invokeDesktop('set_active_project', { id });
}

export async function getActiveProjectContext() {
  if (!isDesktopApp()) return { project: null, bundle: null, deployDir: '' };
  return await invokeDesktop('get_active_project_context');
}

export async function activateProjectBundle(projectId, bundleDir, version) {
  return await invokeDesktop('activate_project_bundle', { projectId, bundleDir, version });
}
