import type { LocalSite, LocalSiteInput, Site, SiteType } from '@shared/types'
import { normalizeSourceUrl, sourceHostName } from '@shared/url'
import { readStore, writeStore } from './store'

const NAME = 'local-sites'

function shortHash(input: string): string {
  let h = 0x811c9dc5
  for (let i = 0; i < input.length; i++) {
    h ^= input.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return (h >>> 0).toString(36)
}

/** 没写类型时按地址猜一个保守值：苹果CMS 采集接口常见的两种路径 */
function guessType(api: string): SiteType {
  return /provide\/vod|api\.php|\/vod\//i.test(api) ? 3 : 4
}

export async function listLocalSites(): Promise<LocalSite[]> {
  const list = await readStore<LocalSite[]>(NAME, [])
  return Array.isArray(list) ? list : []
}

function save(list: LocalSite[]): void {
  writeStore(NAME, list)
}

/** 手动源的 key 与地址绑定，重新添加同一地址会命中同一条 */
export function localSiteKey(api: string): string {
  return `manual_${shortHash(normalizeSourceUrl(api))}`
}

export async function addLocalSite(input: LocalSiteInput): Promise<LocalSite> {
  const api = normalizeSourceUrl(input.api)
  if (!api) throw new Error('接口地址不能为空')
  const list = await listLocalSites()
  if (list.some((s) => normalizeSourceUrl(s.api) === api)) {
    throw new Error('该影视源已存在')
  }

  const type = input.type ?? guessType(api)
  const now = Date.now()
  const site: LocalSite = {
    key: localSiteKey(api),
    name: input.name?.trim() || sourceHostName(api),
    type,
    api,
    searchable: input.searchable ?? true,
    filterable: input.filterable ?? (type === 3 || type === 4),
    playerType: 1,
    header: input.header,
    timeout: input.timeout,
    enabled: input.enabled ?? true,
    createdAt: now,
    updatedAt: now
  }
  list.push(site)
  save(list)
  return site
}

export async function updateLocalSite(
  key: string,
  patch: Partial<Omit<LocalSiteInput, 'api'>> & { api?: string }
): Promise<LocalSite> {
  const list = await listLocalSites()
  const index = list.findIndex((s) => s.key === key)
  if (index < 0) throw new Error('影视源不存在')

  const current = list[index]
  const nextApi = patch.api === undefined ? current.api : normalizeSourceUrl(patch.api)
  if (!nextApi) throw new Error('接口地址不能为空')
  if (list.some((s, i) => i !== index && normalizeSourceUrl(s.api) === nextApi)) {
    throw new Error('该影视源已存在')
  }

  const next: LocalSite = {
    ...current,
    ...patch,
    key: nextApi === current.api ? current.key : localSiteKey(nextApi),
    api: nextApi,
    name: patch.name === undefined ? current.name : patch.name.trim() || current.name,
    type: patch.type ?? current.type,
    updatedAt: Date.now()
  }
  list[index] = next
  save(list)
  return next
}

export async function removeLocalSite(key: string): Promise<void> {
  const list = await listLocalSites()
  save(list.filter((s) => s.key !== key))
}

export function localToSite(local: LocalSite): Site {
  return {
    ...local,
    quickSearch: local.searchable,
    sourceId: 'local',
    sourceName: '手动添加',
    origin: 'manual'
  }
}
