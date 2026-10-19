import { reactive } from 'vue';
import { profileState } from './config/profile.js';

export const connection = reactive({
  mode: 'remote',
  bundleDir: profileState.bundleDir,
  mediaDir: profileState.mediaDir || profileState.bundleDir,
  deployDir: profileState.deployDir,
  installDir: profileState.installDir,
  remoteHost: '',
  remotePort: 22,
  remoteUser: 'root',
  remotePassword: '',
  remoteKey: ''
});

export function syncConnectionFromProfile() {
  if (profileState.bundleDir) {
    connection.bundleDir = profileState.bundleDir;
  }
  if (profileState.mediaDir) {
    connection.mediaDir = profileState.mediaDir;
  }
  if (profileState.deployDir) {
    connection.deployDir = profileState.deployDir;
  }
  if (profileState.installDir) {
    connection.installDir = profileState.installDir;
  }
}
