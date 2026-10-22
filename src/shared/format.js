/** 文件大小：小于 1 GB 保留一位小数，达到 1 GB 保留两位。 */
export function formatSize(bytes) {
  const value = Number(bytes) || 0;
  if (value >= 1024 * 1024 * 1024) return `${(value / 1024 / 1024 / 1024).toFixed(2)} GB`;
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
  if (value >= 1024) return `${(value / 1024).toFixed(0)} KB`;
  return `${value} B`;
}

/** 镜像 ID：去掉 sha256 前缀后取前 12 位。 */
export function formatImageId(id) {
  const value = String(id || '').replace(/^sha256:/, '');
  return value ? value.slice(0, 12) : '-';
}

/** 完整时间：年-月-日 时:分:秒。 */
export function formatTimestamp(ms) {
  if (!ms) return '-';
  const date = new Date(Number(ms));
  if (Number.isNaN(date.getTime())) return '-';
  const pad = (value) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/** 简短时间：月/日 时:分，用于列表摘要。 */
export function formatShortTimestamp(value) {
  if (!value) return '未记录';
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  }).format(new Date(value));
}
