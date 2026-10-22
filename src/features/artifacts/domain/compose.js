/** 部署配置检查结果的展示分组与文案，供对话框与表格面板共用。 */

export function composeKindLabel(kind) {
  if (kind === 'compose') return 'compose 清单';
  if (kind === 'yaml') return 'YAML';
  return '文本';
}

/** 镜像核对的对象不同，同一个状态在界面上的叫法也不同。 */
export function imageAvailableLabel(imageCheck) {
  return imageCheck === 'remote' ? '目标机可用' : '本机可用';
}

export function summaryCards(images = [], available) {
  const count = (status) => images.filter((image) => image.status === status).length;
  return [
    { key: 'ready', label: '就绪', count: count('ready'), tone: 'ok' },
    { key: 'to_import', label: '待导入', count: count('to_import'), tone: 'warn' },
    { key: 'missing', label: '缺失', count: count('missing'), tone: 'danger' },
    { key: 'unconfirmed', label: '未确认', count: count('unconfirmed'), tone: 'unknown' },
    { key: 'loaded', label: available, count: count('loaded'), tone: 'ok' }
  ];
}

export function dockerColumnLabel(imageCheck) {
  if (imageCheck === 'remote') return '目标机 Docker';
  if (imageCheck === 'unavailable') return 'Docker 状态';
  return '本机 Docker';
}

export function sourceLabel(source) {
  if (source === 'images.json') return '镜像包清单';
  if (source === 'profile') return '部署配置';
  if (source === 'filename') return '文件名匹配';
  return '来源未知';
}

export function statusLabel(status, available) {
  if (status === 'ready') return '就绪';
  if (status === 'to_import') return '待导入';
  if (status === 'loaded') return available;
  if (status === 'missing') return '缺失';
  if (status === 'unconfirmed') return '未确认';
  return status;
}

export function statusTone(status) {
  if (status === 'ready' || status === 'loaded') return 'success';
  if (status === 'to_import') return 'warning';
  if (status === 'missing') return 'danger';
  return 'info';
}

export function dockerTone(row, imageCheck) {
  if (row?.dockerPresent) return 'success';
  return imageCheck === 'unavailable' ? 'info' : 'warning';
}

export function dockerLabel(row, imageCheck) {
  if (row?.dockerPresent) return '已加载';
  return imageCheck === 'unavailable' ? '未确认' : '未加载';
}

/** 配置检查里的体积：0 显示 0 B，其余按量级保留小数。 */
export function composeSizeText(bytes) {
  const size = Number(bytes || 0);
  if (!size) return '0 B';
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
  return `${(size / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/** 配置正文按行拆分，超出行数上限时提示只显示前若干行。 */
export function contentLines(content, maxLines) {
  const raw = String(content || '').split('\n');
  return {
    lines: raw.slice(0, maxLines).map((text, index) => ({
      no: index + 1,
      text: text || ' ',
      highlight: /^\s*(-\s*)?image:\s*\S/.test(text)
    })),
    truncated: raw.length > maxLines
  };
}

export function composeFooterHint(view) {
  if (!view) return '';
  if (view.kind !== 'compose') return '查看部署配置内容。';
  if (!view.images?.length) return '此配置没有需要检查的镜像。';
  if (view.imageCheck === 'unavailable') {
    return '镜像状态未确认，请检查 Docker 或目标机连接后重新检查。离线包存在不代表镜像已导入。';
  }
  return `${view.imageCheck === 'remote' ? '目标机' : '本机'}已有镜像可直接使用。离线包用于导入缺失镜像或保留备份。`;
}
