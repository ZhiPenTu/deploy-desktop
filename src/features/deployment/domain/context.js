export function validInstallDir(value, remote = false) {
  const path = String(value || '').trim();
  if (!path || /[\n\r\0]/.test(path)) return false;
  const windows = /^[A-Za-z]:[\\/]/.test(path);
  if (!path.startsWith('/') && !(windows && !remote)) return false;
  const parts = (windows ? path.slice(3).split(/[\\/]/) : path.split('/'))
    .filter((part) => part && part !== '.');
  return parts.length > 0 && !parts.includes('..');
}

export function createDeploymentContext({
  connection,
  remote,
  bundle,
  components,
  backup,
  action,
  label,
  imageDir = '',
  allowedComponents = []
}) {
  if (!validInstallDir(connection.installDir, remote)) throw new Error('请填写有效的部署目录，不能使用根目录或上级路径。');
  if (remote && (!connection.remoteHost?.trim() || !connection.remoteUser?.trim())) {
    throw new Error('请先选择目标主机并补齐登录信息。');
  }
  if (remote && (!Number.isInteger(connection.remotePort) || connection.remotePort < 1 || connection.remotePort > 65535)) {
    throw new Error('请填写有效的 SSH 端口。');
  }
  if (!bundle?.dir?.trim()) throw new Error('请先选择并检查交付包。');
  if (!['first', 'incremental'].includes(action)) throw new Error('请选择部署方式。');
  if (!Array.isArray(components) || (action === 'incremental' && !components.length)) {
    throw new Error('请至少选择一个更新组件。');
  }
  const allowed = new Set(Array.isArray(allowedComponents) ? allowedComponents : []);
  if (components.some((key) => !allowed.has(key))) throw new Error('更新组件无效，请重新选择交付包。');
  return Object.freeze({
    remote,
    host: connection.remoteHost,
    port: connection.remotePort,
    user: connection.remoteUser,
    keyPath: connection.remoteKey || null,
    bundleDir: bundle.dir.trim(),
    installDir: connection.installDir.trim(),
    imageDir,
    version: bundle.version,
    components: Object.freeze([...components]),
    backup,
    action,
    label
  });
}

export function contextTarget(context) {
  return context.remote
    ? { host: context.host, port: context.port, user: context.user, keyPath: context.keyPath }
    : null;
}
