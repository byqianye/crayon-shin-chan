/** Spider 返回的防盗链头只用于对应媒体 origin，不能发往其他站点。 */
const grants = new Map<string, { at: number; header: Record<string, string> }>()
const TTL = 30 * 60_000
const ALLOWED = /^(user-agent|referer|origin|cookie|authorization)$/i

function mediaOrigin(url: string): string | undefined {
  try {
    const target = new URL(url)
    return /^https?:$/.test(target.protocol) ? target.origin : undefined
  } catch { return undefined }
}

export function registerMediaHeaders(url: string, input?: Record<string, string>): void {
  const origin = mediaOrigin(url)
  if (!origin) return
  const header = Object.fromEntries(Object.entries(input ?? {}).filter(
    ([key, value]) => ALLOWED.test(key) && typeof value === 'string' && !/[\r\n]/.test(value)
  ))
  grants.delete(origin)
  if (!Object.keys(header).length) return
  grants.set(origin, { at: Date.now(), header })
  while (grants.size > 64) grants.delete(grants.keys().next().value!)
}

export function mediaHeadersFor(url: string): Record<string, string> | undefined {
  const origin = mediaOrigin(url)
  if (!origin) return undefined
  const grant = grants.get(origin)
  if (!grant) return undefined
  if (Date.now() - grant.at >= TTL) { grants.delete(origin); return undefined }
  return grant.header
}
