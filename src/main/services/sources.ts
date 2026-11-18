import { randomUUID } from 'node:crypto'
import type { ConfigSource, ParsedConfig } from '@shared/types'
import { normalizeSourceUrl } from '@shared/url'
import { parseConfig, loadConfigSource } from './config'
import { extractMultiRepo, loadMultiRepo } from './multirepo'
import { getSettings } from './settings'
import { addSourceTombstone, removeSourceTombstone } from './tombstones'
import { readStore, writeStore } from './store'

const NAME = 'sources'

/** 已解析配置的内存缓存：sourceId -> ParsedConfig */
const parsedCache = new Map<string, ParsedConfig>()
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
  for (const source of list) {
    if (!source.enabled || !source.raw) continue
    try {
      parsedCache.set(
        source.id,
        parseConfig(source.raw, { sourceId: source.id, sourceName: source.name })
      )
    } catch (err) {
      console.warn(`[sources] 回填 ${source.name} 失败`, err)
    }
  }
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
  if (patch.url) {
    next.url = normalizeSourceUrl(patch.url)
    next.kind = /^https?:\/\//i.test(next.url) ? 'url' : 'file'
    next.repos = undefined
  }
  list[idx] = next
  await saveSources(list)
  if (patch.enabled === false) parsedCache.delete(id)
  return next
}

export async function removeSource(id: string): Promise<void> {
  const list = await listSources()
  const source = list.find((s) => s.id === id)
  if (source) await addSourceTombstone(source.url)
  await saveSources(list.filter((s) => s.id !== id))
  parsedCache.delete(id)
}

export async function syncSource(id: string): Promise<ConfigSource> {
  const list = await listSources()
  const idx = list.findIndex((s) => s.id === id)
  if (idx < 0) throw new Error('配置源不存在')
  const source = list[idx]
  const settings = await getSettings()

  const next: ConfigSource = { ...source }
  try {
    const parsed = await loadConfigSource(source, settings.timeout)
    const refs = extractMultiRepo(parsed.raw)
    if (refs.length && !parsed.sites.length) {
      // 多仓清单：逐个拉取子仓库，合并成一份可直接 hydrate 的配置
      const multi = await loadMultiRepo(source, refs, settings.timeout)
      parsedCache.set(source.id, multi.parsed)
      next.kind = 'multirepo'
      next.repos = multi.repos
      next.raw = {
        name: multi.parsed.name,
        sites: multi.parsed.sites,
        parses: multi.parsed.parses,
        lives: multi.parsed.lives
      }
      next.siteCount = multi.parsed.sites.length
      next.usableSiteCount = multi.parsed.sites.filter((s) => !s.unsupported).length
      next.liveCount = multi.parsed.lives.length
      next.parseCount = multi.parsed.parses.length
    } else {
      parsedCache.set(source.id, parsed)
      next.kind = source.kind === 'file' ? 'file' : 'url'
      next.repos = undefined
      next.raw = parsed.raw
      next.siteCount = parsed.sites.length
      next.usableSiteCount = parsed.sites.filter((s) => !s.unsupported).length
      next.liveCount = parsed.lives.length
      next.parseCount = parsed.parses.length
    }
    next.lastSync = Date.now()
    next.error = undefined
    next.name = source.name || parsed.name
    next.notice = parsed.notice
  } catch (err) {
    next.error = (err as Error).message
    // 同步失败但本地有缓存时保留可用数据，界面只提示不中断
  }
  list[idx] = next
  await saveSources(list)
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
