import { artifactSizeText, artifactTimeText } from './releases.js';

/** 部署向导的制品选择：准备动作文案、选中判定与展示格式。 */
export function prepareActionLabel(candidate) {
  if (candidate?.kind === 'release') return '下载、解压并校验';
  if (candidate?.kind === 'archive') return '解压并校验';
  return '校验交付包';
}

export function prepareActionHint(candidate) {
  if (candidate?.kind === 'release') return '将下载所选版本，完成完整性校验后解压到本机。';
  if (candidate?.kind === 'archive') return '将交付包解压到本机，并检查内容是否完整。';
  return '检查交付包是否完整，以及部署清单是否可用。';
}

/** 同一条版本记录是否就是当前已选的制品。 */
export function isChosenVersion(candidate, sourceId, item) {
  if (!item || !candidate) return false;
  if (candidate.sourceId !== sourceId) return false;
  const version = String(item.version || '').trim();
  return Boolean(version) && candidate.version === version;
}

export function versionLabel(value) {
  return value && value !== 'manual' ? value : '未标记版本';
}

/** 交付包体积：向导里未记录时说明「未知大小」，与制品列表的「-」区分。 */
export function sizeLabel(bytes) {
  const value = Number(bytes || 0);
  return Number.isFinite(value) && value > 0 ? artifactSizeText(value) : '未知大小';
}

/** 构建时间：向导里未记录时说明「未知时间」，与制品列表的「-」区分。 */
export function timeLabel(ms) {
  return Number(ms || 0) > 0 ? artifactTimeText(ms) : '未知时间';
}
