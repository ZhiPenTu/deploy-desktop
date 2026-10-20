import { invokeDesktop, isDesktopApp } from '../desktop.js';

export async function listConnectionInventory() {
  if (!isDesktopApp()) {
    return { credentials: [], hosts: [], activeHostId: '' };
  }
  return await invokeDesktop('list_connection_inventory');
}

export async function saveSshCredential(credential) {
  return await invokeDesktop('save_ssh_credential', { credential });
}

export async function deleteSshCredential(id) {
  return await invokeDesktop('delete_ssh_credential', { id });
}

export async function saveManagedHost(host) {
  return await invokeDesktop('save_managed_host', { host });
}

export async function deleteManagedHost(id) {
  return await invokeDesktop('delete_managed_host', { id });
}

export async function setActiveManagedHost(id) {
  return await invokeDesktop('set_active_managed_host', { id });
}

export async function testManagedHostConnection(id) {
  return await invokeDesktop('test_managed_host_connection', { id });
}

export async function setupManagedHostAccess(id) {
  return await invokeDesktop('setup_managed_host_access', { id });
}
