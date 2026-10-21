// Stable public facade: feature code imports desktop operations only from this file.
export { isDesktopApp, selectLocalPath } from './desktop.js';

export {
  listArtifactSources,
  saveArtifactSource,
  deleteArtifactSource,
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
  getDeploymentProfile,
  checkAppUpdate,
  installAppUpdate
} from './commands/settings.js';
