import { randomUUID } from 'node:crypto'
import type { ConfigSource, ParsedConfig } from '@shared/types'
import { normalizeSourceUrl, resolveSourceAddress } from '@shared/url'
import { parseConfig, loadConfigSource } from './config'
import { extractMultiRepo, loadMultiRepo } from './multirepo'
import { getSettings } from './settings'
import { addSourceTombstone, removeSourceTombstone } from './tombstones'
import { readStore, writeStore } from './store'

const NAME = 'sources'

/** 已解析配置的内存缓存：sourceId -> ParsedConfig */
const parsedCache = new Map<string, ParsedConfig>()
const syncRequests = new Map<string, Promise<ConfigSource>>()
let hydrated = false

export async function listSources(): Promise<ConfigSource[]> {
  const list = await readStore<ConfigSource[]>(NAME, [])
  return Array.isArray(list) ? list : []
}

async function saveSources(list: ConfigSource[]): Promise<void> {
  writeStore(NAME, list)
}

/** 启动时用上次成功的原始配置回填缓存，做到「首屏不等网络」 */
export async function hydrate(): Promise<void> {
  if (hydrated) return
  hydrated = true
  const list = await listSources()
  let changed = false
  for (const source of list) {
    if (!source.raw) continue
    try {
      const parsed = parseConfig(source.raw, {
        sourceId: source.id,
        sourceName: source.name,
        baseUrl: source.url
      })
      parsedCache.set(source.id, parsed)
      // 升级后配置的可用性会变化（例如 csp_* 从「不支持」变成 Spider 可运行）。
      // 这里用本地 raw 回填计数，避免设置页一直显示旧版本留下的「可用 0」。
      const usable = parsed.sites.filter((s) => !s.unsupported).length
      if (
        source.siteCount !== parsed.sites.length ||
        source.usableSiteCount !== usable ||
        source.liveCount !== parsed.lives.length ||
        source.parseCount !== parsed.parses.length
      ) {
        source.siteCount = parsed.sites.length
        source.usableSiteCount = usable
        source.liveCount = parsed.lives.length
        source.parseCount = parsed.parses.length
        changed = true
      }
    } catch (err) {
      console.warn(`[sources] 回填 ${source.name} 失败`, err)
    }
  }
  if (changed) await saveSources(list)
}

export function getParsed(sourceId: string): ParsedConfig | undefined {
  return parsedCache.get(sourceId)
}

export async function addSource(input: {
  name?: string
  url: string
  kind?: 'url' | 'file'
}): Promise<ConfigSource> {
  const url = normalizeSourceUrl(input.url)
  if (!url) throw new Error('地址不能为空')
  const kind = input.kind ?? (/^https?:\/\//i.test(url) ? 'url' : 'file')
  const list = await listSources()
  if (list.some((s) => normalizeSourceUrl(s.url) === url)) throw new Error('该配置源已存在')
  // 用户主动重新添加同一地址，视为解除之前的删除墓碑
  await removeSourceTombstone(url)

  const source: ConfigSource = {
    id: randomUUID(),
    name: (input.name || '').trim() || guessName(url, kind),
    url,
    kind,
    enabled: true,
    lastSync: 0,
    siteCount: 0,
    liveCount: 0,
    parseCount: 0
  }
  list.push(source)
  await saveSources(list)
  return source
}

function guessName(url: string, kind: 'url' | 'file'): string {
  if (kind === 'file') {
    const base = url.split(/[\\/]/).pop() ?? '本地配置'
    return base.replace(/\.json$/i, '')
  }
  try {
    return new URL(url).hostname
  } catch {
    return '远程配置'
  }
}

export async function updateSource(
  id: string,
  patch: Partial<Pick<ConfigSource, 'name' | 'url' | 'enabled'>>
): Promise<ConfigSource> {
  const list = await listSources()
  const idx = list.findIndex((s) => s.id === id)
  if (idx < 0) throw new Error('配置源不存在')
  const next: ConfigSource = { ...list[idx], ...patch }
  next.url = patch.url ? normalizeSourceUrl(patch.url) : list[idx].url
  if (next.url !== list[idx].url) {
    next.kind = /^https?:\/\//i.test(next.url) ? 'url' : 'file'
    next.repos = undefined
    next.raw = undefined
    next.lastSync = 0
    next.siteCount = 0
    next.usableSiteCount = 0
    next.liveCount = 0
    next.parseCount = 0
    next.error = undefined
    next.notice = undefined
    parsedCache.delete(id)
  }
  list[idx] = next
  await saveSources(list)
  return next
}

export async function removeSource(id: string): Promise<void> {
  const list = await listSources()
  const source = list.find((s) => s.id === id)
  if (source) await addSourceTombstone(source.url)
  await saveSources(list.filter((s) => s.id !== id))
  parsedCache.delete(id)
}

export function syncSource(id: string): Promise<ConfigSource> {
  const existing = syncRequests.get(id)
  if (existing) return existing
  const pending = fetchSource(id).finally(() => syncRequests.delete(id))
  syncRequests.set(id, pending)
  return pending
}

async function fetchSource(id: string): Promise<ConfigSource> {
  const source = (await listSources()).find((s) => s.id === id)
  if (!source) throw new Error('配置源不存在')
  const { url, name, kind } = source
  const settings = await getSettings()

  const update: Partial<ConfigSource> = {}
  let fetched: ParsedConfig | undefined
  try {
    const parsed = await loadConfigSource(source, settings.timeout)
    const refs = extractMultiRepo(parsed.raw).map((ref) => ({ ...ref, url: resolveSourceAddress(ref.url, url) }))
    if (refs.length && !parsed.sites.length) {
      // 多仓清单：逐个拉取子仓库，合并成一份可直接 hydrate 的配置
      const multi = await loadMultiRepo(source, refs, settings.timeout)
      fetched = multi.parsed
      update.kind = 'multirepo'
      update.repos = multi.repos
      update.raw = {
        name: multi.parsed.name,
        sites: multi.parsed.sites,
        parses: multi.parsed.parses,
        lives: multi.parsed.lives,
        headers: multi.parsed.headers,
        flags: multi.parsed.flags,
        home: multi.parsed.home
      }
    } else {
      fetched = parsed
      update.kind = kind === 'file' ? 'file' : 'url'
      update.repos = undefined
      update.raw = parsed.raw
    }
    update.siteCount = fetched.sites.length
    update.usableSiteCount = fetched.sites.filter((s) => !s.unsupported).length
    update.liveCount = fetched.lives.length
    update.parseCount = fetched.parses.length
    update.lastSync = Date.now()
    update.error = undefined
    update.notice = parsed.notice
  } catch (err) {
    // 同步失败但本地有缓存时保留可用数据，界面只提示不中断
    update.error = (err as Error).message
  }
  // 拉取期间用户可以修改或删除源：只把同步结果合入当前记录。
  const current = await listSources()
  const idx = current.findIndex((s) => s.id === id)
  if (idx < 0) throw new Error('配置源不存在')
  const latest = current[idx]
  if (latest.url !== url) throw new Error('配置源地址已修改，请重新同步')
  const next = { ...latest, ...update }
  if (fetched) {
    if (latest.name !== name) {
      for (const site of fetched.sites) {
        if (site.sourceName === name) site.sourceName = latest.name
      }
    }
    parsedCache.set(id, fetched)
  }
  current[idx] = next
  await saveSources(current)
  return next
}

export async function syncAll(onlyEnabled = true): Promise<ConfigSource[]> {
  const list = await listSources()
  const targets = list.filter((s) => (onlyEnabled ? s.enabled : true))
  const results: ConfigSource[] = []
  // 顺序同步即可：站点数量有限，避免同时打太多源触发风控
  for (const source of targets) {
    results.push(await syncSource(source.id))
  }
  return results
}
