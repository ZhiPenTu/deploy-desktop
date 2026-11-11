export function safeText(value) {
  return errorMessage(value)
    .replace(/-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g, '[私钥已隐藏]')
    .replace(/\b(?:https?|ssh):\/\/[^\s]+/gi, '[地址已隐藏]')
    .replace(
      /((?:["']?)[\w-]*(?:password|passwd|token|secret|authorization)["']?\s*[:=]\s*)(?:"(?:\\.|[^"])*"|'(?:\\.|[^'])*'|[^\s,;]+)/gi,
      '$1[已隐藏]'
    )
    .replace(/((?:--password|--token)\s+)(?:"[^"]*"|'[^']*'|\S+)/gi, '$1[已隐藏]')
    .replace(/(Bearer\s+)[\w.-]+/gi, '$1[已隐藏]');
}

function errorMessage(error) {
  const text = typeof error === 'string' ? error : error?.message;
  return String(text || '');
}

function actionableReason(text) {
  for (const line of text.split('\n').map((item) => item.trim()).filter(Boolean)) {
    if (
      !/(未在等待时间内就绪|未就绪|未创建|不一致|未声明|未包含|缺少|不足|尚未|未安装|未运行|未找到|被占用|占用|正在使用|无法替换|无法写入|未能恢复|恢复失败|自动恢复失败|备份目录|临时旧文件)/.test(
        line
      )
    )
      continue;
    const cleaned = line
      .replace(/^Error:\s*/i, '')
      .replace(/^应用更新失败[：:]\s*/, '')
      .replace(/^更新未完成[：:]\s*/, '')
      .replace(/[。.]\s*$/, '')
      .trim();
    if (cleaned) return cleaned;
  }
  return '';
}

export function failureText(error, action, { remote } = {}) {
  const text = errorMessage(error).trim().replace(/^Error:\s*/i, '');
  const reason = actionableReason(text);
  if (reason) return `${action}失败：${reason}。`;
  const authentication =
    /publickey|authentication failed|ssh[^\n]{0,20}认证失败|认证失败[^\n]{0,20}ssh|密码错误|私钥口令/i.test(
      text
    );
  const permission = /权限|denied|拒绝访问/i.test(text);
  if (
    authentication ||
    (permission && /^(连接检查|配置免密登录)$/.test(action))
  )
    return `${action}失败，请检查主机登录权限和凭证后重试。`;
  if (permission) {
    return remote === false
      ? `${action}失败，请检查目标目录或文件权限后重试。`
      : `${action}失败，请检查目标主机登录权限或目标目录权限后重试。`;
  }
  if (/Docker|docker|Compose|compose/.test(text))
    return `${action}失败，请确认 Docker 和 Compose 正常运行，并检查服务配置。`;
  if (/镜像|image/i.test(text)) return `${action}失败，请补齐所需镜像后重新比对。`;
  if (/目录|文件|路径|bundle|交付包/.test(text)) return `${action}失败，请检查交付包与部署目录后重试。`;
  return `${action}失败，请检查目标主机连接后重试。`;
}
