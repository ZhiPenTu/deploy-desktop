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
  const lines = text
    .split('\n')
    .map((item) => item.trim())
    .filter(Boolean);
  const clean = (line) =>
    line
      .replace(/^Error:\s*/i, '')
      .replace(/^[\u4e00-\u9fff]{1,12}失败[：:]\s*/, '')
      .replace(/^Compose 操作失败[：:]\s*/i, '')
      .replace(/[。.]\s*$/, '')
      .trim();
  for (const line of lines) {
    // 原始命令输出与替换详情属于诊断内容，只在执行详情中展示，不作为主提示。
    if (/^(原始输出|替换详情|原始错误)[：:]/.test(line)) continue;
    if (
      !/(未在等待时间内就绪|未就绪|未创建|不一致|未声明|未包含|缺少|缺失|不足|尚未|未安装|未运行|未找到|已不存在|被占用|占用|正在使用|部署目录外|重叠|无法替换|无法写入|无法挂载|无法访问|不匹配|地址池|已用完|同名容器|依赖服务|不支持|未能恢复|恢复失败|自动恢复失败|备份目录|临时旧文件)/.test(
        line
      )
    )
      continue;
    const cleaned = clean(line);
    if (cleaned) return cleaned;
  }
  return '';
}

// 后端说明本身已是面向用户的中文句子时直接采用，避免落到“请检查目标主机连接”。
function fallbackSentence(text) {
  for (const line of text
    .split('\n')
    .map((item) => item.trim())
    .filter(Boolean)) {
    if (/^(原始输出|替换详情|原始错误)[：:]/.test(line)) continue;
    if (/连接超时|网络连通性|连接失败/.test(line)) continue;
    if (!/[\u4e00-\u9fff]/.test(line) || line.length > 200) continue;
    if (!/请|无法|不能|已停止|已阻止|不一致/.test(line)) continue;
    return line
      .replace(/^[\u4e00-\u9fff]{1,12}失败[：:]\s*/, '')
      .replace(/[。.]\s*$/, '')
      .trim();
  }
  return '';
}

function dockerFailureReason(text) {
  if (
    /cannot connect to the docker daemon|docker daemon is not running|error during connect|dial unix .*docker\.sock|connection refused/i.test(
      text
    )
  )
    return '无法连接 Docker 服务，请确认 Docker 已启动后重试';
  if (
    /permission denied while trying to connect to the docker daemon|got permission denied while trying to connect/i.test(
      text
    )
  )
    return '当前账户没有 Docker 访问权限，请调整权限后重试';
  if (/no such image|pull access denied|manifest unknown|manifest for .* not found/i.test(text))
    return '目标主机缺少所需镜像，请先准备镜像后重试';
  if (/conflict\. the container name|container name .* is already in use|name .* already in use/i.test(text))
    return '已存在同名容器，请停止或清理旧容器后重试';
  if (/port is already allocated|address already in use|bind for .* failed/i.test(text))
    return '所需端口已被占用，请停止占用该端口的服务后重试';
  if (/exec format error|no matching manifest for|does not match the specified platform/i.test(text))
    return '镜像架构与目标主机不一致，请准备与目标架构匹配的镜像后重试';
  if (/no space left on device/i.test(text)) return '目标磁盘空间不足，请清理空间后重试';
  if (/read-only file system/i.test(text)) return '目标目录不可写，请更换部署目录后重试';
  return '';
}

export function failureText(error, action, { remote } = {}) {
  const text = errorMessage(error).trim().replace(/^Error:\s*/i, '');
  const reason = actionableReason(text);
  if (reason) return `${action}失败：${safeText(reason)}。`;
  const authentication =
    /publickey|authentication failed|ssh[^\n]{0,20}认证失败|认证失败[^\n]{0,20}ssh|密码错误|私钥口令/i.test(
      text
    );
  // 挂载失败常带有 denied 字样，必须先于权限判断，否则会被当成目录权限问题。
  const mount = /无法挂载|mounts? denied/i.test(text);
  if (mount) {
    return `${action}失败，请确认部署目录位于本机磁盘目录，避免使用网络共享目录后重试。`;
  }
  const dockerReason = dockerFailureReason(text);
  if (dockerReason) return `${action}失败，${dockerReason}。`;
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
    return `${action}失败，Docker 未能完成操作，请查看执行详情或运行日志中的具体原因后重试。`;
  if (/镜像|image/i.test(text)) return `${action}失败，请补齐所需镜像后重新比对。`;
  if (/目录|文件|路径|bundle|交付包/.test(text)) return `${action}失败，请检查交付包与部署目录后重试。`;
  const sentence = fallbackSentence(text);
  if (sentence) return `${action}失败：${safeText(sentence)}。`;
  return `${action}失败，请检查目标主机连接后重试。`;
}
