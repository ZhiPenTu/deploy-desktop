/** 服务清单比对与容器状态的展示文案。 */
export function changeLabel(value) {
  return (
    { added: '新增', changed: '有变化', unchanged: '一致', removed: '制品中移除' }[value] || '未确认'
  );
}

export function stateLabel(value) {
  return (
    {
      running: '运行中',
      exited: '已停止',
      created: '已创建',
      restarting: '重启中',
      not_created: '未创建',
      missing: '未创建'
    }[value] || '未运行'
  );
}

export function artifactStatusLabel(status, hasDeployment) {
  if (status === 'present') return '已存在';
  if (status === 'missing') return hasDeployment ? '缺失' : '待写入';
  if (status === 'incomplete') return '不完整';
  if (status === 'invalid') return '路径异常';
  return '未核对';
}

export function artifactStatusTone(status, hasDeployment) {
  if (status === 'present') return 'success';
  if (status === 'missing') return hasDeployment ? 'warning' : 'info';
  if (status === 'incomplete' || status === 'invalid') return 'warning';
  return 'info';
}
