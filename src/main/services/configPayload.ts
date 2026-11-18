import { decodeResponse, request } from './http'

/** TVBox 生态的配置服务器经常按 UA 返回不同内容（浏览器拿到导航页，客户端拿到配置） */
export const CONFIG_UA = 'okhttp/3.12.0'

/** 肥猫公开的解密服务；加密配置的算法是 hex-tag-aes，先走社区服务，后续可换本地实现 */
const DECRYPT_ENDPOINT = 'http://肥猫.net/api/parse'

export type ConfigWrapper = 'json' | 'encrypted' | 'image'

export interface LoadedConfigPayload {
  raw: unknown
  wrapper: ConfigWrapper
}

function isImageBuffer(buffer: Buffer, contentType: string): boolean {
  if (/^image\//i.test(contentType)) return true
  if (buffer.length >= 2 && buffer[0] === 0xff && buffer[1] === 0xd8) return true // JPEG
  if (
    buffer.length >= 8 &&
    buffer[0] === 0x89 &&
    buffer[1] === 0x50 &&
    buffer[2] === 0x4e &&
    buffer[3] === 0x47
  ) {
    return true // PNG
  }
  return false
}

/**
 * 去掉 JSONC 的 // 与 /* *\/ 注释。
 * 必须识别字符串状态，否则 https:// 里的 // 会被当成注释截断。
 */
function stripJsonComments(text: string): string {
  let out = ''
  let inString = false
  let escaped = false
  let i = 0
  while (i < text.length) {
    const ch = text[i]
    const next = text[i + 1]
    if (inString) {
      out += ch
      if (escaped) escaped = false
      else if (ch === '\\') escaped = true
      else if (ch === '"') inString = false
      i += 1
      continue
    }
    if (ch === '"') {
      inString = true
      out += ch
      i += 1
      continue
    }
    if (ch === '/' && next === '/') {
      while (i < text.length && text[i] !== '\n') i += 1
      continue
    }
    if (ch === '/' && next === '*') {
      i += 2
      while (i < text.length && !(text[i] === '*' && text[i + 1] === '/')) i += 1
      i += 2
      continue
    }
    out += ch
    i += 1
  }
  return out
}

function parseJsonText(text: string): unknown {
  const cleaned = stripJsonComments(text.replace(/^\uFEFF/, '').trim())
  // 顶层可能是对象（单仓）或数组（多仓清单）
  const firstObject = cleaned.indexOf('{')
  const firstArray = cleaned.indexOf('[')
  const start =
    firstObject < 0 ? firstArray : firstArray < 0 ? firstObject : Math.min(firstObject, firstArray)
  const end = Math.max(cleaned.lastIndexOf('}'), cleaned.lastIndexOf(']'))
  if (start < 0 || end <= start) throw new Error('配置内容不是合法 JSON')
  const body = cleaned.slice(start, end + 1).replace(/,\s*([}\]])/g, '$1')
  return JSON.parse(body)
}

/** 饭太硬这类源：JPEG/PNG 图片尾部追加 Base64 配置 */
function extractImageTail(buffer: Buffer): string | null {
  let end = buffer.lastIndexOf(Buffer.from([0xff, 0xd9]))
  if (end < 0) end = buffer.lastIndexOf(Buffer.from('IEND'))
  if (end < 0) return null
  const tail = buffer.slice(end + 2)
  const base64 = tail.toString('ascii').replace(/[^A-Za-z0-9+/=]/g, '')
  if (base64.length < 64) return null
  const decoded = Buffer.from(base64, 'base64').toString('utf8')
  const start = decoded.indexOf('{')
  if (start < 0) return null
  return decoded.slice(start)
}

/** 把 2423…2324 形态的密文交给肥猫公开解密接口，返回其中的配置对象 */
async function decryptHexTag(cipher: string, timeout: number): Promise<unknown> {
  const res = await request(DECRYPT_ENDPOINT, {
    method: 'POST',
    timeout: Math.max(timeout, 30),
    header: {
      'Content-Type': 'application/json',
      'X-Requested-With': 'fetch',
      'User-Agent': CONFIG_UA
    },
    body: JSON.stringify({ url: '', content: cipher })
  })
  const payload = JSON.parse(decodeResponse(res)) as Record<string, unknown>
  if (payload.ok === false) throw new Error(String(payload.error ?? '加密配置解密失败'))
  const json = payload.json ?? payload.config
  if (typeof json === 'string') return JSON.parse(json)
  if (json && typeof json === 'object') return json
  throw new Error('解密服务没有返回配置内容')
}

/**
 * 统一加载配置源：兼容普通 JSON、图片尾 Base64、hex-tag-aes 密文。
 * 所有远程请求都用 okhttp UA，这是 TVBox 配置服务器识别客户端的常见约定。
 */
export async function loadConfigPayload(
  url: string,
  timeout = 20
): Promise<LoadedConfigPayload> {
  const res = await request(url, {
    timeout,
    header: { 'User-Agent': CONFIG_UA }
  })
  const contentType = res.headers['content-type'] ?? ''

  if (isImageBuffer(res.buffer, contentType)) {
    const tail = extractImageTail(res.buffer)
    if (!tail) throw new Error('图片配置里没有找到 Base64 内容')
    return { raw: parseJsonText(tail), wrapper: 'image' }
  }

  const text = decodeResponse(res)
  const trimmed = text.trim()
  if (/^2423[0-9a-f]+2324$/i.test(trimmed) || /^2423[0-9a-f]{16,}/i.test(trimmed)) {
    return { raw: await decryptHexTag(trimmed, timeout), wrapper: 'encrypted' }
  }

  return { raw: parseJsonText(text), wrapper: 'json' }
}
