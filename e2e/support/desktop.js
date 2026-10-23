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
    artifactReleases = [],
    dockerImages = [],
    offlineDir = '',
    offlinePackages = [],
    projects = [],
    activeProjectId = '',
    dockerInstallation = { os: 'linux', installed: true, version: 'Docker version 27.0.0' },
    localEnv = null,
    remoteEnv = null,
    dialogPath = '',
    preparedBundle = null,
    bundlePlan = null,
    bundleConfig = null,
    deploymentReview = null,
    containers = [],
    bundle = null,
    remoteImageBundle = null
  } = {}
) {
  await page.addInitScript(({
    initialHosts,
    initialCredentials,
    initialSources,
    initialTokens,
    initialReleases,
    initialImages,
    initialOfflineDir,
    initialPackages,
    initialProjects,
    initialActiveProjectId,
    initialDockerInstallation,
    initialLocalEnv,
    initialRemoteEnv,
    initialDialogPath,
    initialPreparedBundle,
    initialBundlePlan,
    initialBundleConfig,
    initialDeploymentReview,
    initialContainers,
    initialBundle,
    initialRemoteImageBundle
  }) => {
    const state = {
      hosts: structuredClone(initialHosts),
      credentials: structuredClone(initialCredentials),
      sources: structuredClone(initialSources),
      tokens: structuredClone(initialTokens),
      releases: structuredClone(initialReleases),
      images: structuredClone(initialImages),
      offlineDir: initialOfflineDir,
      packages: structuredClone(initialPackages),
      projects: structuredClone(initialProjects),
      activeProjectId: initialActiveProjectId || initialProjects[0]?.id || '',
      dockerInstallation: structuredClone(initialDockerInstallation),
      localEnv: structuredClone(initialLocalEnv),
      remoteEnv: structuredClone(initialRemoteEnv),
      dialogPath: initialDialogPath,
      preparedBundle: structuredClone(initialPreparedBundle),
      bundlePlan: structuredClone(initialBundlePlan),
      bundleConfig: structuredClone(initialBundleConfig),
      deploymentReview: structuredClone(initialDeploymentReview),
      containers: structuredClone(initialContainers),
      boundBundle: structuredClone(initialBundle),
      remoteImageBundle: structuredClone(initialRemoteImageBundle),
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

    const projectStore = () => ({
      schemaVersion: 1,
      activeProjectId: state.activeProjectId,
      projects: state.projects.map((item) => ({ ...item }))
    });

    window.isTauri = true;
    window.__deployTestCalls = [];
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
        window.__deployTestCalls.push({ command, args });
        switch (command) {
          case 'check_docker_installation':
            if (!state.dockerInstallation) throw '无法读取 Docker 安装状态，请重新检测。';
            return structuredClone(state.dockerInstallation);
          case 'open_docker_installation':
            return null;
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
            return projectStore();
          case 'set_active_project':
            state.activeProjectId = args.id;
            return projectStore();
          case 'get_active_project_context': {
            const project = state.projects.find((item) => item.id === state.activeProjectId) || null;
            return {
              project: project ? { ...project } : null,
              bundle: state.boundBundle ? { ...state.boundBundle } : null,
              deployDir: project?.installDir || ''
            };
          }
          case 'plugin:dialog|open':
            return state.dialogPath || null;
          case 'get_containers_status':
            return state.containers.map((item) => ({ ...item }));
          case 'get_remote_containers_status':
            return state.containers.map((item) => ({ ...item }));
          case 'inspect_remote_image_bundle':
            if (!state.remoteImageBundle) throw '当前操作暂不可用，请在桌面应用中重试。';
            return structuredClone(state.remoteImageBundle);
          case 'prepare_local_artifact_bundle':
            if (!state.preparedBundle) throw '当前操作暂不可用，请在桌面应用中重试。';
            return { ...state.preparedBundle };
          case 'activate_project_bundle': {
            const index = state.projects.findIndex((item) => item.id === args.projectId);
            if (index < 0) throw '请先在「项目管理」中新建项目并选择目标主机';
            state.projects[index] = {
              ...state.projects[index],
              lastBundleDir: args.bundleDir,
              lastVersion: args.version
            };
            state.boundBundle = { dir: args.bundleDir, version: args.version };
            return projectStore();
          }
          case 'inspect_deployment_review':
            if (!state.deploymentReview) throw '当前操作暂不可用，请在桌面应用中重试。';
            return structuredClone(state.deploymentReview);
          case 'inspect_bundle_plan':
            if (!state.bundlePlan) throw '当前操作暂不可用，请在桌面应用中重试。';
            return structuredClone(state.bundlePlan);
          case 'inspect_bundle_config':
            if (!state.bundleConfig) throw '当前操作暂不可用，请在桌面应用中重试。';
            return structuredClone(state.bundleConfig);
          case 'check_local_env':
            if (!state.localEnv) throw '当前操作暂不可用，请在桌面应用中重试。';
            return structuredClone(state.localEnv);
          case 'check_remote_env':
            if (!state.remoteEnv) throw '当前操作暂不可用，请在桌面应用中重试。';
            return structuredClone(state.remoteEnv);
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
          case 'list_cached_artifact_releases':
            return { page: 1, hasMore: false, items: [] };
          case 'list_artifact_releases':
            return {
              page: args.page || 1,
              hasMore: false,
              items: state.releases.map((item) => ({ ...item }))
            };
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
    initialReleases: artifactReleases,
    initialImages: dockerImages,
    initialOfflineDir: offlineDir,
    initialPackages: offlinePackages,
    initialProjects: projects,
    initialActiveProjectId: activeProjectId,
    initialDockerInstallation: dockerInstallation,
    initialLocalEnv: localEnv,
    initialRemoteEnv: remoteEnv,
    initialDialogPath: dialogPath,
    initialPreparedBundle: preparedBundle,
    initialBundlePlan: bundlePlan,
    initialBundleConfig: bundleConfig,
    initialDeploymentReview: deploymentReview,
    initialContainers: containers,
    initialBundle: bundle,
    initialRemoteImageBundle: remoteImageBundle
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
