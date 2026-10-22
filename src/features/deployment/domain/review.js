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
