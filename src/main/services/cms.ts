import type { Site } from '@shared/types'
import * as cheerio from 'cheerio'
import { parseLooseJson } from './http'

/**
 * 苹果CMS(MacCMS V10) 接口地址构造。
 * 同时支持「基地址 + 参数」和「模板地址（含 {ac} {wd} {pg} {t} {ids} 占位符）」两种写法。
 */

export interface CmsQuery {
  ac?: 'list' | 'detail' | 'videolist'
  wd?: string
  pg?: number | string
  t?: number | string
  ids?: number | string
  h?: string
}

const SLOT_KEYS: Record<string, keyof CmsQuery> = {
  ac: 'ac', wd: 'wd', kw: 'wd', key: 'wd', keyword: 'wd', pg: 'pg', page: 'pg',
  t: 't', type: 't', typeid: 't', class: 't', ids: 'ids', id: 'ids', h: 'h'
}

export function buildCmsUrl(site: Site, query: CmsQuery): string {
  const slots = new Set<keyof CmsQuery>()
  const expanded = site.api.replace(/\{(ac|wd|kw|key|keyword|pg|page|t|type|typeid|class|ids|id|h)\}/gi, (_slot, name: string) => {
    const key = SLOT_KEYS[name.toLowerCase()]
    slots.add(key)
    return encodeURIComponent(String(query[key] ?? ''))
  })
  const url = new URL(expanded)
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && !slots.has(key as keyof CmsQuery)) url.searchParams.set(key, String(value))
  }
  return url.toString()
}

/**
 * 分类浏览用的地址，和 buildCmsUrl 的区别是 t 可以传多个。
 *
 * 苹果CMS 把同一个大类拆成一堆叶子分类（电影 → 动作片/喜剧片/爱情片…），
 * 而**父分类 id 查不出任何东西**（实测 t=1 返回 0 条），只能落到叶子上。
 *
 * 注意：**不要指望把多个叶子 id 并列进同一次请求**。实测重复的 t 参数是
 * 「最后一个生效」而不是 OR——`t=6&t=7` 的 total 等于 t=7 的 total、返回的也全是
 * 喜剧片，`t=6,7` 则只认第一个。所以调用方（category.ts）对每个子分类单独发一次请求，
 * 拿回来自己合并；这里传多个 id 只是给模板式站点留个兜底。
 * 模板式站点只有一个 {t} 槽位，塞不下多个值，只取第一个叶子。
 */
export function buildCmsBrowseUrl(site: Site, ac: string, types: string[], pg: number): string {
  const api = site.api ?? ''
  // 只要地址里有任何占位符就算模板式。不能只认 {t}：像
  // `...?ac={ac}&pg={pg}&wd={wd}` 这种没有 {t} 的模板，如果走下面的「追加参数」
  // 分支，模板里的字面量 {ac}/{pg}/{wd} 会和新参数同时留在 URL 上，
  // 服务端读到哪一个完全看它先取哪个，等于随机。
  const templated = /\{(ac|wd|kw|key|keyword|pg|page|t|type|typeid|class|ids|id|h)\}/.test(api)
  if (templated) {
    return buildCmsUrl(site, { ac: ac as CmsQuery['ac'], t: types[0] ?? '', pg })
  }
  const params = new URLSearchParams()
  params.set('ac', ac)
  for (const t of types) if (t) params.append('t', t)
  params.set('pg', String(pg))
  const sep = api.includes('?') ? (api.endsWith('?') || api.endsWith('&') ? '' : '&') : '?'
  return `${api}${sep}${params.toString()}`
}

/** 归一化苹果CMS 返回，容忍 code 为 1 / "1" / "ok" 等写法 */
export function isCmsOk(payload: unknown): boolean {
  if (!payload || typeof payload !== 'object') return false
  const code = (payload as Record<string, unknown>).code
  if (code === undefined || code === null || code === '') return true
  const s = String(code).toLowerCase()
  return s === '1' || s === 'true' || s === 'ok' || s === 'success' || s === '200'
}

export function asArray<T = unknown>(value: unknown): T[] {
  if (Array.isArray(value)) return value as T[]
  if (value && typeof value === 'object') return [value as T]
  return []
}

/** XML 采集接口与 JSON 接口统一为同一种响应，避免各链路支持范围不同。 */
export function parseCmsPayload(text: string): Record<string, unknown> {
  if (!/^\s*\uFEFF?\s*</.test(text)) {
    const payload = parseLooseJson<Record<string, unknown>>(text)
    if (!payload || typeof payload !== 'object' || Array.isArray(payload) ||
      !['list', 'class', 'code'].some((key) => key in payload)) throw new Error('接口未返回 CMS 数据')
    return payload
  }
  const $ = cheerio.load(text, { xml: true })
  if (!$('rss').length || (!$('rss > list').length && !$('rss > class').length)) {
    throw new Error('接口未返回 CMS XML 数据')
  }
  const list = $('rss > list')
  const fields: Record<string, string> = { id: 'vod_id', name: 'vod_name', pic: 'vod_pic',
    note: 'vod_remarks', year: 'vod_year', area: 'vod_area', actor: 'vod_actor',
    director: 'vod_director', type: 'type_name', score: 'vod_score', des: 'vod_content' }
  return {
    code: 1,
    page: list.attr('page'), pagecount: list.attr('pagecount'), total: list.attr('recordcount'),
    class: $('rss > class > ty').toArray().map((el) => ({
      type_id: $(el).attr('id'), type_name: $(el).text().trim(), type_pid: $(el).attr('pid') ?? '0'
    })),
    list: list.children('video').toArray().map((el) => {
      const node = $(el)
      const item: Record<string, unknown> = {}
      for (const [tag, key] of Object.entries(fields)) item[key] = node.children(tag).text().trim()
      const lines = node.find('dl > dd').toArray()
      item.vod_play_from = lines.map((line) => $(line).attr('flag') ?? '').join('$$$')
      item.vod_play_url = lines.map((line) => $(line).text().trim()).join('$$$')
      return item
    })
  }
}
