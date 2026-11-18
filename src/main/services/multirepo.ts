import type {
  ConfigSource,
  LiveSource,
  MultiRepoEntry,
  ParsedConfig,
  ParseItem,
  Site
} from '@shared/types'
import { fetchJson } from './http'
import { parseConfig } from './config'

export interface MultiRepoRef {
  name: string
  url: string
}

/** 兼容 TVBox 多仓常见的 urls / repos / 数组三种写法 */
export function extractMultiRepo(raw: unknown): MultiRepoRef[] {
  let entries: unknown = null
  if (Array.isArray(raw)) {
    entries = raw
  } else if (raw && typeof raw === 'object') {
    const obj = raw as Record<string, unknown>
    entries = Array.isArray(obj.urls) ? obj.urls : Array.isArray(obj.repos) ? obj.repos : null
  }
  if (!Array.isArray(entries)) return []

  return entries.flatMap((item): MultiRepoRef[] => {
    if (typeof item === 'string') return item ? [{ name: item, url: item }] : []
    if (!item || typeof item !== 'object') return []
    const obj = item as Record<string, unknown>
    const url = String(obj.url ?? '').trim()
    if (!url) return []
    return [{ name: String(obj.name ?? url).trim() || url, url }]
  })
}

export interface MultiRepoResult {
  parsed: ParsedConfig
  repos: MultiRepoEntry[]
}

/**
 * 逐个拉取子仓库，失败隔离：一个仓库挂掉不影响其他仓库。
 * 合并后的 sites 保留每个子仓库的 sourceName，界面能看出站点来自哪个仓库。
 */
export async function loadMultiRepo(
  source: Pick<ConfigSource, 'id' | 'name'>,
  refs: MultiRepoRef[],
  timeout: number
): Promise<MultiRepoResult> {
  const sites: Site[] = []
  const parses: ParseItem[] = []
  const lives: LiveSource[] = []
  const repos: MultiRepoEntry[] = []

  for (const ref of refs) {
    try {
      const raw = await fetchJson(ref.url, { timeout })
      const parsed = parseConfig(raw, {
        sourceId: source.id,
        sourceName: ref.name,
        defaults: { timeout }
      })
      sites.push(...parsed.sites)
      parses.push(...parsed.parses)
      lives.push(...parsed.lives)
      repos.push({ name: ref.name, url: ref.url, ok: true, siteCount: parsed.sites.length })
    } catch (err) {
      repos.push({
        name: ref.name,
        url: ref.url,
        ok: false,
        error: (err as Error).message
      })
    }
  }

  return {
    parsed: {
      name: source.name,
      sites,
      parses,
      lives,
      flags: [],
      headers: [],
      raw: { multiRepo: refs.map((ref) => ref.url) }
    },
    repos
  }
}
