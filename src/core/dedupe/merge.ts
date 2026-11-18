import type { SiteSearchResult, Vod } from '@shared/types'
import {
  buildDedupeKey,
  extractSeason,
  extractYear,
  isCommentary,
  normalizeName,
  scoreCandidate,
  titleSimilarity,
  typeBucket
} from './normalize'

/** 聚合视图里的一条：同一部影片可能来自多个站点 */
export interface MergedVod extends Vod {
  /** 拥有这部影片的全部站点 key（第一个是代表） */
  allSiteKeys: string[]
  /** 除了代表之外，其它可切换的站点 */
  alternatives: Vod[]
}

export interface MergeOptions {
  keyword?: string
  filterCommentary?: boolean
  /**
   * 上一轮已经上屏的顺序（去重键 → 名次），只有分类浏览翻页时会传。
   * 传了就把老卡片原样排在前面、新一页的追在后面。
   */
  previousOrder?: Map<string, number>
}

/**
 * 同一个站点可能对同一部影片返回好几条（国语版 / 粤语版 / 重复收录），
 * 这里每个站点只留一条：先看信息全不全，再优先用最干净的那个片名。
 */
function collapseBySite(list: Vod[]): Vod[] {
  const sorted = [...list].sort((a, b) => {
    const sa = scoreCandidate(a)
    const sb = scoreCandidate(b)
    if (sa !== sb) return sb - sa
    return (a.vod_name ?? '').length - (b.vod_name ?? '').length
  })
  const seen = new Set<string>()
  const out: Vod[] = []
  for (const vod of sorted) {
    if (seen.has(vod.siteKey)) continue
    seen.add(vod.siteKey)
    out.push(vod)
  }
  return out
}

/** 片名与关键词的贴合度，越小越靠前 */
function matchRank(name: string, keyword: string): number {
  const n = normalizeName(name)
  const k = normalizeName(keyword)
  if (!k) return 3
  if (n === k) return 0
  if (n.startsWith(k)) return 1
  if (n.includes(k)) return 2
  return 3
}

/**
 * 两个精确去重键不同的条目是否仍应合并。
 *
 * 只在前面的硬约束全部满足时才启用相似度兜底：
 * 同内容类型、同季数、年份相同（或至少一方缺失但片名完全一致）。
 * 这是为了不把「射雕英雄传 1983 / 2008 / 2017」这类同名不同版误合。
 */
function canMerge(a: Vod, b: Vod): boolean {
  const bucketA = typeBucket(a.type_name)
  const bucketB = typeBucket(b.type_name)
  // 网页规则站拿不到 type_name（归为 other），不能因此把同一部片拆成两条；
  // 只有两边都是已知类型且不同才拒绝合并。
  if (bucketA !== 'other' && bucketB !== 'other' && bucketA !== bucketB) return false
  if (extractSeason(a.vod_name) !== extractSeason(b.vod_name)) return false

  const yearA = extractYear(a.vod_year, a.vod_name)
  const yearB = extractYear(b.vod_year, b.vod_name)
  if (yearA && yearB && yearA !== yearB) return false
  if (!yearA || !yearB) {
    return normalizeName(a.vod_name) === normalizeName(b.vod_name)
  }
  return titleSimilarity(a.vod_name, b.vod_name) >= 0.92
}

/**
 * 把各站点的结果按「类型 + 片名 + 季数 + 年份」合并成一部影片一条。
 *
 * 代表条目取信息最全的那条（有海报 > 有备注 > 有评分），所以合并后的卡片
 * 不会因为先到的站点没给海报就变成灰块。
 */
export function mergeResults(
  results: SiteSearchResult[],
  options: MergeOptions = {}
): { merged: MergedVod[]; hidden: number } {
  const { keyword = '', filterCommentary = true } = options
  const exact = new Map<string, Vod[]>()
  let hidden = 0

  for (const result of results) {
    for (const vod of result.list) {
      const name = (vod.vod_name ?? '').trim()
      if (!name) continue
      if (filterCommentary && isCommentary(name, vod.type_name)) {
        hidden++
        continue
      }
      const key = buildDedupeKey(vod)
      const bucket = exact.get(key)
      if (!bucket) exact.set(key, [vod])
      else bucket.push(vod)
    }
  }

  // 精确键分组后，对「类型/季数/年份」一致、只是片名写法略有差异的组做一次兜底合并。
  const groups: { key: string; list: Vod[] }[] = []
  for (const [key, list] of exact) {
    const candidate = list[0]
    const target = groups.find((group) => canMerge(group.list[0], candidate))
    if (target) {
      target.list.push(...list)
    } else {
      groups.push({ key, list: [...list] })
    }
  }

  const merged: MergedVod[] = []
  for (const group of groups) {
    const bySite = collapseBySite(group.list)
    const [best, ...rest] = bySite
    merged.push({ ...best, allSiteKeys: bySite.map((v) => v.siteKey), alternatives: rest })
  }

  merged.sort((a, b) => {
    const ra = matchRank(a.vod_name, keyword)
    const rb = matchRank(b.vod_name, keyword)
    if (ra !== rb) return ra - rb
    if (a.allSiteKeys.length !== b.allSiteKeys.length) {
      return b.allSiteKeys.length - a.allSiteKeys.length
    }
    const sa = scoreCandidate(a)
    const sb = scoreCandidate(b)
    if (sa !== sb) return sb - sa
    return a.vod_name.localeCompare(b.vod_name, 'zh-CN')
  })

  // 翻页时保住已经上屏的顺序：上一轮在屏幕上的按原名次排在前面，新出来的追在后面。
  const previous = options.previousOrder
  if (previous?.size) {
    const known: MergedVod[] = []
    const fresh: MergedVod[] = []
    for (const vod of merged) {
      const key = buildDedupeKey(vod)
      if (previous.has(key)) known.push(vod)
      else fresh.push(vod)
    }
    known.sort(
      (a, b) => (previous.get(buildDedupeKey(a)) ?? 0) - (previous.get(buildDedupeKey(b)) ?? 0)
    )
    merged.length = 0
    merged.push(...known, ...fresh)
  }

  return { merged, hidden }
}
