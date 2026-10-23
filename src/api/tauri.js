// Stable public facade: feature code imports desktop operations only from this file.
export { isDesktopApp, selectLocalPath, reportClientEvent, setCommandTrace } from './desktop.js';

export {
  listArtifactSources,
  saveArtifactSource,
  deleteArtifactSource,
  listArtifactTokens,
  saveArtifactToken,
  deleteArtifactToken,
  fetchArtifactRelease,
  listArtifactReleases,
  listCachedArtifactReleases,
  downloadArtifact,
  openArtifactBundle,
  openArtifactBundleVersion,
  deleteLocalBundle,
  openArtifactRunUrl,
  prepareLocalArtifactBundle
} from './commands/artifacts.js';

export {
  listConnectionInventory,
  saveSshCredential,
  deleteSshCredential,
  saveManagedHost,
  deleteManagedHost,
  setActiveManagedHost,
  testManagedHostConnection,
  setupManagedHostAccess
} from './commands/connections.js';

export {
  installOfflineDockerEngine,
  installRemoteDockerEngine,
  loadOfflineImages,
  exportDeploymentImages,
  inspectBundlePlan,
  inspectBundleConfig,
  inspectDeploymentReview,
  updateRemoteDeployment,
  updateLocalDeployment,
  installBundle,
  onRemoteUpdateProgress
} from './commands/deployment.js';

export {
  scanLocalImageBundle,
  inspectRemoteImageBundle,
  uploadRemoteImageBundle,
  importRemoteImageBundle,
  removeRemoteImageFiles,
  getImageLibrarySettings,
  saveImageLibrarySettings,
  listLocalDockerImages,
  pullLocalDockerImage,
  removeLocalDockerImages,
  exportLocalDockerImages,
  importOfflinePackages,
  scanOfflinePackages,
  removeOfflinePackages,
  onImageLibraryProgress,
  onImageManagerProgress
} from './commands/images.js';

export {
  listRegistrySources,
  saveRegistrySource,
  deleteRegistrySource,
  testRegistrySource,
  loadImageManifest,
  pullManifestImagesRemote,
  pullManifestImagesLocal,
  onRegistryProgress
} from './commands/registry.js';

export {
  checkLocalEnv,
  checkRemoteEnv,
  getContainersStatus,
  dockerComposeAction,
  getServiceLogs,
  remoteDockerComposeAction,
  inspectRemoteDeployment,
  getRemoteContainersStatus,
  getRemoteServiceLogs
} from './commands/monitoring.js';

export {
  checkAppUpdate,
  installAppUpdate
} from './commands/settings.js';

export {
  listProjects,
  createProject,
  updateProject,
  deleteProject,
  setActiveProject,
  getActiveProjectContext,
  activateProjectBundle
} from './commands/projects.js';

export {
  readRuntimeLogs,
  clearRuntimeLogs,
  openRuntimeLogDir,
  setRuntimeLogLevel,
  getRuntimeLogSettings,
  setRuntimeLogDir,
  reportClientEvent as reportRuntimeEvent
} from './commands/logs.js';
