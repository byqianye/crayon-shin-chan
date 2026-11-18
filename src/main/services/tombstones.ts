import type { Tombstones } from '@shared/types'
import { normalizeSourceUrl } from '@shared/url'
import { readStore, writeStore } from './store'

const NAME = 'tombstones'
const EMPTY: Tombstones = { sourceUrls: [], siteKeys: [] }

export async function listTombstones(): Promise<Tombstones> {
  const data = await readStore<Tombstones>(NAME, EMPTY)
  return {
    sourceUrls: Array.isArray(data?.sourceUrls) ? data.sourceUrls : [],
    siteKeys: Array.isArray(data?.siteKeys) ? data.siteKeys : []
  }
}

async function save(data: Tombstones): Promise<void> {
  writeStore(NAME, data)
}

/** 删除配置源时记一笔，重新添加同一地址时会自动解除 */
export async function addSourceTombstone(url: string): Promise<void> {
  const target = normalizeSourceUrl(url)
  if (!target) return
  const data = await listTombstones()
  if (!data.sourceUrls.includes(target)) {
    data.sourceUrls.push(target)
    await save(data)
  }
}

/** 忽略订阅里的某个站点：后续同步不再把它带回来 */
export async function addSiteTombstone(siteKey: string): Promise<void> {
  if (!siteKey) return
  const data = await listTombstones()
  if (!data.siteKeys.includes(siteKey)) {
    data.siteKeys.push(siteKey)
    await save(data)
  }
}

export async function removeSiteTombstone(siteKey: string): Promise<void> {
  const data = await listTombstones()
  data.siteKeys = data.siteKeys.filter((key) => key !== siteKey)
  await save(data)
}

export async function removeSourceTombstone(url: string): Promise<void> {
  const target = normalizeSourceUrl(url)
  const data = await listTombstones()
  data.sourceUrls = data.sourceUrls.filter((item) => item !== target)
  await save(data)
}

export async function clearTombstones(): Promise<void> {
  await save({ ...EMPTY })
}
