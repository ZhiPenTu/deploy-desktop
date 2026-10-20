export function safeText(value) {
  return String(value || '')
    .replace(/-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g, '[私钥已隐藏]')
    .replace(/\b(?:https?|ssh):\/\/[^\s]+/gi, '[地址已隐藏]')
    .replace(
      /((?:["']?)[\w-]*(?:password|passwd|token|secret|authorization)["']?\s*[:=]\s*)(?:"(?:\\.|[^"])*"|'(?:\\.|[^'])*'|[^\s,;]+)/gi,
      '$1[已隐藏]'
    )
    .replace(/((?:--password|--token)\s+)(?:"[^"]*"|'[^']*'|\S+)/gi, '$1[已隐藏]')
    .replace(/(Bearer\s+)[\w.-]+/gi, '$1[已隐藏]');
}

export function failureText(error, action) {
  const text = String(error || '');
  if (/权限|denied|认证|authentication|publickey/i.test(text))
    return `${action}失败，请检查主机登录权限和凭证后重试。`;
  if (/Docker|docker|Compose|compose/.test(text))
    return `${action}失败，请确认 Docker 和 Compose 正常运行，并检查服务配置。`;
  if (/镜像|image/i.test(text)) return `${action}失败，请补齐所需镜像后重新比对。`;
  if (/目录|文件|路径|bundle|交付包/.test(text)) return `${action}失败，请检查交付包与部署目录后重试。`;
  return `${action}失败，请检查目标主机连接后重试。`;
}
