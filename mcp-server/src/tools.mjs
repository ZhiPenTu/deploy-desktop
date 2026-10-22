import { wrapTool } from './tools/runtime.mjs';
import { registerProfileTools } from './tools/profile.mjs';
import { registerHostsTools } from './tools/hosts.mjs';
import { registerRegistriesTools } from './tools/registries.mjs';
import { registerManifestsTools } from './tools/manifests.mjs';
import { registerImagesTools } from './tools/images.mjs';
import { registerBundlesTools } from './tools/bundles.mjs';
import { registerOperationsTools } from './tools/operations.mjs';
import { registerUpdatesTools } from './tools/updates.mjs';
import { registerLogsTools } from './tools/logs.mjs';

export function registerTools(server) {
  const register = (name, definition, handler) => server.registerTool(name, definition, wrapTool(handler));
  registerProfileTools(register);
  registerHostsTools(register);
  registerRegistriesTools(register);
  registerManifestsTools(register);
  registerImagesTools(register);
  registerBundlesTools(register);
  registerOperationsTools(register);
  registerUpdatesTools(register);
  registerLogsTools(register);
}
