/**
 * 源类型探测（纯函数，不做网络请求）。
 *
 * 主进程负责把 URL 内容拉下来，这里只根据内容判断类型，
 * 这样规则可以单测，也不会把网络逻辑渗进领域层。
 */
export type DetectedSourceKind =
  | 'cms'
  | 'tvbox'
  | 'multirepo'
  | 'm3u'
  | 'html'
  | 'encrypted'
  | 'image'
  | 'unknown'

export interface DetectResult {
  kind: DetectedSourceKind
  name?: string
  siteCount?: number
  repoCount?: number
}

function repoEntries(raw: unknown): unknown[] | null {
  if (Array.isArray(raw)) return raw
  if (raw && typeof raw === 'object') {
    const obj = raw as Record<string, unknown>
    if (Array.isArray(obj.urls)) return obj.urls
    if (Array.isArray(obj.repos)) return obj.repos
  }
  return null
}

export function detectSourceContent(text: string): DetectResult {
  const body = (text ?? '').replace(/^\uFEFF/, '').trim()
  if (!body) return { kind: 'unknown' }
  if (body.startsWith('#EXTM3U')) return { kind: 'm3u' }
  if (/<rss\b/i.test(body) && /<(list|class)\b/i.test(body)) return { kind: 'cms' }
  // 肥猫 / 潇洒这类 hex-tag-aes 加密配置
  if (/^2423[0-9a-f]{16,}/i.test(body)) return { kind: 'encrypted' }

  let json: unknown
  try {
    json = JSON.parse(body)
  } catch {
    // 不是 JSON 的 http 内容按网页规则站处理；空壳或错误页交给用户再判断
    return /<html|<body|<!doctype/i.test(body) ? { kind: 'html' } : { kind: 'unknown' }
  }

  if (Array.isArray(json)) {
    const entries = json.filter(
      (item): item is { url: string; name?: string } =>
        Boolean(item) &&
        typeof item === 'object' &&
        typeof (item as Record<string, unknown>).url === 'string'
    )
    if (entries.length && entries.length === json.length) {
      return { kind: 'multirepo', repoCount: entries.length }
    }
    return { kind: 'unknown' }
  }

  if (json && typeof json === 'object') {
    const obj = json as Record<string, unknown>
    if (Array.isArray(obj.sites)) {
      return {
        kind: 'tvbox',
        name: typeof obj.name === 'string' ? obj.name : undefined,
        siteCount: obj.sites.length
      }
    }
    const repos = repoEntries(obj)
    if (repos) {
      return {
        kind: 'multirepo',
        name: typeof obj.name === 'string' ? obj.name : undefined,
        repoCount: repos.length
      }
    }
    // 苹果CMS 的接口响应：ac=list 有 class，ac=videolist/detail 有 list
    if (Array.isArray(obj.class) || Array.isArray(obj.list)) return { kind: 'cms' }
  }

  return { kind: 'unknown' }
}

function isImageBytes(buffer: Uint8Array): boolean {
  if (buffer.length >= 2 && buffer[0] === 0xff && buffer[1] === 0xd8) return true
  return (
    buffer.length >= 8 &&
    buffer[0] === 0x89 &&
    buffer[1] === 0x50 &&
    buffer[2] === 0x4e &&
    buffer[3] === 0x47
  )
}

/** 主进程拿到原始响应后调用：图片伪装配置与文本配置都能识别 */
export function detectSourceBuffer(buffer: Uint8Array, contentType = ''): DetectResult {
  if (/^image\//i.test(contentType) || isImageBytes(buffer)) return { kind: 'image' }
  const text = new TextDecoder('utf-8').decode(buffer)
  return detectSourceContent(text)
}
