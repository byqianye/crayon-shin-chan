/**
 * 源地址规范化，用于「重复添加」与「墓碑」比对。
 *
 * 只做不影响请求语义的归一：协议/主机小写、去 fragment、去默认端口、
 * 去末尾多余斜杠。查询串保持原样——它可能包含接口必需参数。
 */
export function normalizeSourceUrl(raw: string): string {
  const input = (raw ?? '').trim()
  if (!input) return ''
  // Windows 本地路径（含 UNC）：不能交给 URL 解析——盘符会被当成 scheme，
  // 中文路径会被百分号编码，最终文件打不开。
  if (/^[a-zA-Z]:[\\/]/.test(input) || input.startsWith('\\\\')) {
    return input.replace(/[\\/]+$/, '')
  }
  try {
    const url = new URL(input)
    url.hash = ''
    url.hostname = url.hostname.toLowerCase()
    if (
      (url.protocol === 'http:' && url.port === '80') ||
      (url.protocol === 'https:' && url.port === '443')
    ) {
      url.port = ''
    }
    if (url.pathname.length > 1) url.pathname = url.pathname.replace(/\/+$/, '')
    return url.toString()
  } catch {
    return input.replace(/\/+$/, '').toLowerCase()
  }
}

/** 订阅内的相对资源以订阅地址为基准；模板占位符仍由对应适配器替换。 */
export function resolveSourceAddress(raw: string, baseUrl?: string): string {
  if (!raw || !baseUrl || !/^https?:\/\//i.test(baseUrl) || /^[a-z][\w+.-]*:/i.test(raw)) return raw
  return new URL(raw, baseUrl).toString().replace(/%7B/gi, '{').replace(/%7D/gi, '}')
}

/** 从地址里取一个可读的默认名称 */
export function sourceHostName(raw: string): string {
  if (/^[a-zA-Z]:[\\/]/.test(raw) || raw.startsWith('\\\\')) {
    return raw.split(/[\\/]/).pop()?.replace(/\.json$/i, '') || '本地配置'
  }
  try {
    return new URL(raw).hostname || '手动影视源'
  } catch {
    return '手动影视源'
  }
}
