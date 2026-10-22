/** 制品版本的展示文案：未标注版本时给出可读提示，而不是空白或内部标记。 */
export function versionText(item) {
  const version = String(item?.version || '').trim();
  if (!version || version === 'manual') return '未标注版本';
  return version;
}

/** 版本行在列表中的稳定标识，用于加载状态与去重。 */
export function releaseKey(item) {
  return `${item?.runId || 0}-${item?.artifactName || ''}-${item?.version || ''}`;
}

export function localStatusText(row) {
  return row?.localDir ? '已下载' : '未下载';
}

/** 目录路径过长时只保留最后两级，避免撑破列表单元格。 */
export function shortPath(dir) {
  const raw = String(dir || '');
  const parts = raw.split('/').filter(Boolean);
  if (parts.length <= 2) return dir;
  return `.../${parts.slice(-2).join('/')}`;
}

/**
 * 当前部署目录是否落在某个本地制品目录之内。
 * 目录边界按分隔符判断，避免 `/opt/app2` 被误判为在 `/opt/app` 之下；
 * 两侧按原样比较，末尾分隔符不会自动归一化。
 */
export function isBundleDirInUse(currentDir, dir) {
  const current = String(currentDir || '').trim();
  const target = String(dir || '').trim();
  if (!current || !target) return false;
  return current === target || current.startsWith(`${target}/`);
}

/** 制品体积：未记录时显示「-」，小于 1 KB 不带小数，其余保留一位。 */
export function artifactSizeText(bytes) {
  const value = Number(bytes || 0);
  if (!value) return '-';
  const units = ['B', 'KB', 'MB', 'GB'];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

/** 构建时间：未记录时显示「-」，其余按本机时区显示完整时间。 */
export function artifactTimeText(ms) {
  if (!ms) return '-';
  return new Date(Number(ms)).toLocaleString();
}
