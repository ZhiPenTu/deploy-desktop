/**
 * 测试用桌面后端替身：只在浏览器里模拟 Tauri 的 invoke 通道，
 * 供界面测试覆盖有数据时的交互。它不代表真实 Rust、Docker 或 SSH 行为。
 */
export async function useDesktopBackend(
  page,
  {
    hosts = [],
    credentials = [],
    artifactSources = [],
    artifactTokens = [],
    dockerImages = [],
    offlineDir = '',
    offlinePackages = []
  } = {}
) {
  await page.addInitScript(({
    initialHosts,
    initialCredentials,
    initialSources,
    initialTokens,
    initialImages,
    initialOfflineDir,
    initialPackages
  }) => {
    const state = {
      hosts: structuredClone(initialHosts),
      credentials: structuredClone(initialCredentials),
      sources: structuredClone(initialSources),
      tokens: structuredClone(initialTokens),
      images: structuredClone(initialImages),
      offlineDir: initialOfflineDir,
      packages: structuredClone(initialPackages),
      activeHostId: ''
    };
    const callbacks = new Map();
    let nextCallbackId = 1;
    let nextHostId = state.hosts.length + 1;

    // 真实 IPC 每次都会反序列化出新对象，这里同样返回副本，
    // 避免页面收到与上次相同的数组引用而漏掉响应式更新。
    const inventory = () => ({
      credentials: state.credentials.map((item) => ({ ...item })),
      hosts: state.hosts.map((item) => ({ ...item })),
      activeHostId: state.activeHostId
    });

    window.isTauri = true;
    window.__TAURI_INTERNALS__ = {
      transformCallback(callback) {
        const id = nextCallbackId++;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id) {
        callbacks.delete(id);
      },
      convertFileSrc(path) {
        return path;
      },
      async invoke(command, args = {}) {
        switch (command) {
          case 'list_connection_inventory':
            return inventory();
          case 'save_managed_host': {
            const index = state.hosts.findIndex((item) => item.id && item.id === args.host.id);
            const saved = {
              id: args.host.id || `host-${nextHostId++}`,
              kind: args.host.kind,
              name: args.host.name,
              address: args.host.address,
              port: args.host.port,
              credentialId: args.host.credentialId,
              notes: args.host.notes,
              updatedAt: 1_760_000_000_000
            };
            if (index >= 0) state.hosts[index] = saved;
            else state.hosts.push(saved);
            return saved;
          }
          case 'delete_managed_host':
            state.hosts = state.hosts.filter((item) => item.id !== args.id);
            return inventory();
          case 'list_projects':
            return { schemaVersion: 1, activeProjectId: '', projects: [] };
          case 'get_active_project_context':
            return { project: null, bundle: null, deployDir: '' };
          case 'list_artifact_sources':
            return state.sources.map((item) => ({ ...item }));
          case 'save_artifact_source': {
            const saved = {
              ...args.source,
              id: args.source.id || `src-${state.sources.length + 1}`,
              hasToken: Boolean(args.source.tokenId),
              lastVersion: '',
              lastBundleDir: ''
            };
            const index = state.sources.findIndex((item) => item.id === saved.id);
            if (index >= 0) state.sources[index] = saved;
            else state.sources.push(saved);
            return saved;
          }
          case 'list_artifact_tokens':
            return state.tokens.map((item) => ({
              ...item,
              inUse: state.sources.filter((source) => source.tokenId === item.id).length
            }));
          case 'save_artifact_token': {
            const saved = {
              id: args.token.id || `tok-${state.tokens.length + 1}`,
              name: args.token.name,
              hasToken: true,
              inUse: 0,
              createdAt: 1,
              updatedAt: 1
            };
            const index = state.tokens.findIndex((item) => item.id === saved.id);
            if (index >= 0) state.tokens[index] = saved;
            else state.tokens.push(saved);
            return saved;
          }
          case 'delete_artifact_token':
            state.tokens = state.tokens.filter((item) => item.id !== args.id);
            return null;
          case 'list_registry_sources':
            return [];
          case 'get_image_library_settings':
            return { offlineDir: state.offlineDir };
          case 'list_local_docker_images':
            return state.images.map((item) => ({ ...item }));
          case 'scan_offline_packages':
            return {
              offlineDir: state.offlineDir,
              exists: true,
              totalBytes: state.packages.reduce((sum, item) => sum + item.sizeBytes, 0),
              files: state.packages.map((item) => ({ ...item }))
            };
          case 'log_client_event':
            return null;
          default:
            throw '当前操作暂不可用，请在桌面应用中重试。';
        }
      }
    };
  }, {
    initialHosts: hosts,
    initialCredentials: credentials,
    initialSources: artifactSources,
    initialTokens: artifactTokens,
    initialImages: dockerImages,
    initialOfflineDir: offlineDir,
    initialPackages: offlinePackages
  });
}

export const remoteHost = {
  id: 'host-remote',
  kind: 'ssh',
  name: '测试服务器',
  address: '192.0.2.10',
  port: 2222,
  credentialId: 'credential-1',
  notes: '自动化测试数据',
  updatedAt: 1_760_000_000_000
};

export const sshCredential = {
  id: 'credential-1',
  name: '部署账号',
  username: 'deploy',
  privateKeyPath: '/home/tester/.ssh/id_ed25519',
  hasPassword: false,
  updatedAt: 1_760_000_000_000
};
