import { reactive } from 'vue';
import profile from '../profile.json';

export const connection = reactive({
  mode: 'remote',
  bundleDir: profile.bundleDir,
  deployDir: profile.deployDir,
  installDir: profile.installDir,
  remoteHost: '',
  remotePort: 22,
  remoteUser: 'root',
  remotePassword: '',
  remoteKey: ''
});
