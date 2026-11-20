import type { Site } from '@shared/types'
import { asArray, buildCmsUrl, isCmsOk, parseCmsPayload, type CmsQuery } from './cms'
import { fetchText } from './http'
import { resolveHeader } from './headers'
import { getSettings } from './settings'

const DETAIL_TTL = 2 * 60_000
const MAX_DETAILS = 256
const details = new Map<string, { at: number; raw: Record<string, unknown> }>()
const pending = new Map<string, Promise<Record<string, unknown>>>()
let generation = 0

export function clearCmsCache(): void {
  generation++
  details.clear()
  pending.clear()
}

/** 只复用带播放字段的详情；短 TTL 避免长期保存已过期的签名地址。 */
export async function requestCms(site: Site, query: CmsQuery, signal?: AbortSignal): Promise<Record<string, unknown>> {
  signal?.throwIfAborted()
  const url = buildCmsUrl(site, query)
  const header = await resolveHeader(site, url)
  const timeout = site.timeout ?? (await getSettings()).timeout
  signal?.throwIfAborted()
  const identity = JSON.stringify([site.sourceId, site.key, site.api, header, timeout])
  const detailKey = identity + '|' + String(query.ids)
  if (query.ids !== undefined) {
    const cached = details.get(detailKey)
    if (cached && Date.now() - cached.at < DETAIL_TTL) {
      details.delete(detailKey)
      details.set(detailKey, cached)
      return { code: 1, list: [cached.raw] }
    }
    details.delete(detailKey)
  }
  const key = identity + '|' + url
  // 可取消的搜索不能与其他调用共享网络请求，以免互相取消。
  if (!signal && pending.has(key)) return pending.get(key)!
  const gen = generation
  const task = (async () => {
    const payload = parseCmsPayload(await fetchText(url, { header, timeout, signal }))
    if (gen === generation && isCmsOk(payload)) {
      for (const raw of asArray<Record<string, unknown>>(payload.list)) {
        const id = raw.vod_id ?? raw.id
        if (id === undefined || !raw.vod_play_url || JSON.stringify(raw).length > 64 * 1024) continue
        const cacheKey = identity + '|' + String(id)
        details.delete(cacheKey)
        details.set(cacheKey, { at: Date.now(), raw })
        while (details.size > MAX_DETAILS) details.delete(details.keys().next().value!)
      }
    }
    return payload
  })()
  if (!signal) pending.set(key, task)
  try { return await task }
  finally { if (pending.get(key) === task) pending.delete(key) }
}
