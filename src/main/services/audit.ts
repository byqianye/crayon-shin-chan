import type { AuditResult, Site, Vod } from '@shared/types'
import { isCmsOk, asArray } from './cms'
import { requestCms } from './cmsApi'
import { getVodDetail } from './detail'
import { searchSite } from './search'
import { getSite } from './sites'
import { spiderCall } from './spider'

async function countCategories(site: Site): Promise<number> {
  if (site.spiderClass) {
    const payload = await spiderCall<{ class?: unknown }>(site, 'home', { filter: true })
    return asArray(payload?.class).length
  }
  if (site.type !== 3 && site.type !== 4) return 0
  const payload = await requestCms(site, { ac: 'list' })
  if (!isCmsOk(payload)) throw new Error('分类接口返回异常')
  const classes = asArray<unknown>(payload.class)
  return classes.length
}

function firstVod(list: Vod[]): Vod | undefined {
  return list[0]
}

/**
 * 站点三步审计：分类 → 搜索 → 详情。
 * 分类不是所有站都支持，读不到不算失败；搜索与详情才是可用性的硬指标。
 */
export async function auditSite(key: string, probe = '电影'): Promise<AuditResult> {
  const started = Date.now()
  const site = await getSite(key)
  if (!site) throw new Error('站点不存在或已被禁用')

  let categories = 0
  let categoryError: string | undefined
  try {
    categories = await countCategories(site)
  } catch (err) {
    categoryError = (err as Error).message
  }

  const search = await searchSite(site, probe, 5)
  if (!search.ok) {
    return {
      siteKey: site.key,
      siteName: site.name,
      ok: false,
      categories,
      searchHits: 0,
      detailLines: 0,
      episodes: 0,
      ms: Date.now() - started,
      failStage: 'search',
      reason: search.message
    }
  }

  const sample = firstVod(search.list)
  if (!sample) {
    // 接口能通但探针词没命中，算可用；分类错误只作为附注
    return {
      siteKey: site.key,
      siteName: site.name,
      ok: true,
      categories,
      searchHits: 0,
      detailLines: 0,
      episodes: 0,
      ms: Date.now() - started,
      reason: categoryError ? `搜索无结果；${categoryError}` : '搜索无结果'
    }
  }

  try {
    const detail = await getVodDetail(site.key, sample.vod_id)
    const lines = detail.lines.length
    const episodes = detail.lines.reduce((sum, line) => sum + line.episodes.length, 0)
    return {
      siteKey: site.key,
      siteName: site.name,
      ok: lines > 0,
      categories,
      searchHits: search.list.length,
      detailLines: lines,
      episodes,
      ms: Date.now() - started,
      failStage: lines > 0 ? undefined : 'detail',
      reason: lines > 0 ? undefined : '详情没有解析出播放线路'
    }
  } catch (err) {
    return {
      siteKey: site.key,
      siteName: site.name,
      ok: false,
      categories,
      searchHits: search.list.length,
      detailLines: 0,
      episodes: 0,
      ms: Date.now() - started,
      failStage: 'detail',
      reason: (err as Error).message
    }
  }
}
