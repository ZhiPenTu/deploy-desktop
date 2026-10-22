/**
 * 制品源与交付包操作的错误文案。
 * 后端诊断可能带有地址、凭证或主机路径，这里只按失败原因给出可执行的中文提示。
 */
export function artifactFailureText(error, action) {
  const reason = String(error || '');
  if (/令牌|鉴权|权限|401|403|token|unauthorized/i.test(reason)) {
    if (/目录|交付包/.test(action) && !/下载/.test(action)) {
      return '无法读取所选交付包，请确认文件访问权限后重试。';
    }
    return '无法访问制品源，请在设置的“制品设置”中检查访问凭证和权限后重试。';
  }
  if (/非法路径|符号链接|不安全|路径.*越界/.test(reason)) {
    return '交付包包含不安全的路径或链接，已停止解压，请重新获取可信的交付包。';
  }
  if (/缺少|必要文件|内容不完整|bundle\.json|格式无效|不是.*压缩包/.test(reason)) {
    return '交付包格式无效或内容不完整，请选择包含应用文件和部署清单的完整交付包。';
  }
  if (/校验|digest|checksum|md5|sha256/i.test(reason)) {
    return '交付包完整性校验未通过，请重新获取交付包后重试。';
  }
  if (/不存在|not found|no such file/i.test(reason)) {
    return '所选交付包已不存在，请重新选择或下载交付包。';
  }
  if (/超过.*限制|超过.*大小|体积|too large/i.test(reason)) {
    return '交付包超过允许大小，请选择符合大小要求的交付包。';
  }
  if (/网络|连接|超时|network|timeout|connect/i.test(reason)) {
    return '连接制品源失败，请检查网络与下载地址后重试。';
  }
  return `${action}失败，请确认所选制品可用后重试。`;
}
