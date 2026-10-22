import os from 'node:os';
import path from 'node:path';
import { expandHome } from './util.mjs';

/** 桌面端应用配置目录：项目、连接、制品源与令牌等由两端共享的文件都在这里。 */
export function appConfigDir() {
  const override = process.env.DEPLOY_DESKTOP_APP_CONFIG_DIR;
  if (override) return expandHome(override);
  const home = os.homedir();
  if (process.platform === 'darwin') {
    return path.join(home, 'Library', 'Application Support', 'com.deploydesktop.app');
  }
  if (process.platform === 'win32') {
    const appData = process.env.APPDATA || path.join(home, 'AppData', 'Roaming');
    return path.join(appData, 'com.deploydesktop.app');
  }
  return path.join(process.env.XDG_CONFIG_HOME || path.join(home, '.config'), 'com.deploydesktop.app');
}

/**
 * MCP 服务自身的配置目录。
 *
 * 它与桌面端应用配置目录是两回事：前者只放 MCP 的 `mcp.json`，后者是与桌面端共享的
 * 业务配置。两者都用环境变量覆盖，但变量名不同，避免一次覆盖同时改到两处语义。
 */
export function mcpConfigDir() {
  return expandHome(process.env.DEPLOY_DESKTOP_CONFIG_DIR || path.join(os.homedir(), '.deploy-desktop'));
}

/** 未在设置里指定目录时的运行日志位置，与桌面端保持一致。 */
export function defaultLogDir() {
  const home = os.homedir();
  if (process.platform === 'darwin') {
    return path.join(home, 'Library', 'Logs', 'com.deploydesktop.app');
  }
  if (process.platform === 'win32') {
    const localAppData = process.env.LOCALAPPDATA || path.join(home, 'AppData', 'Local');
    return path.join(localAppData, 'com.deploydesktop.app', 'logs');
  }
  const dataHome = process.env.XDG_DATA_HOME || path.join(home, '.local', 'share');
  return path.join(dataHome, 'com.deploydesktop.app', 'logs');
}
