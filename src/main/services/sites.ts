import * as cheerio from 'cheerio'
import type { Site, TestResult } from '@shared/types'
import { normalizeSourceUrl } from '@shared/url'
import { isCmsOk } from './cms'
import { requestCms } from './cmsApi'
import { fetchText } from './http'
import { resolveHeader } from './headers'
export { resolveHeader } from './headers'
import { listLocalSites, localToSite } from './localSites'
import { listTombstones } from './tombstones'
import { getSettings } from './settings'
import { getParsed, listSources } from './sources'
import { classifySpiderFailure, prewarmSpider, spiderCall, spiderSearch } from './spider'
import { readStore, writeStore } from './store'

const NAME = 'sites'

export interface SiteOverride {
  enabled?: boolean
  order?: number
  health?: number
  header?: Record<string, string>
  timeout?: number
  name?: string
}

type OverrideMap = Record<string, SiteOverride>

async function readOverrides(): Promise<OverrideMap> {
  const data = await readStore<OverrideMap>(NAME, {})
  return data && typeof data === 'object' ? data : {}
}

async function writeOverrides(map: OverrideMap): Promise<void> {
  writeStore(NAME, map)
}

/**
 * 汇总所有启用站点：手动添加的源优先，然后是各配置源解析出的站点。
 * 去重规则：同 key 或同接口地址只保留第一条（手动源在配置源之前）；
 * 被用户忽略（墓碑）的站点直接跳过。
 */
export async function getSites(): Promise<Site[]> {
  const sources = await listSources()
  const localSites = await listLocalSites()
  const tombstones = await listTombstones()
  const overrides = await readOverrides()
  const ignored = new Set(tombstones.siteKeys)
  const seenKeys = new Set<string>()
  const manualApis = new Set<string>()
  const out: Site[] = []

  const push = (input: Site): void => {
    if (ignored.has(input.key) || seenKeys.has(input.key)) return
    seenKeys.add(input.key)
    out.push(input)
  }

  for (const local of localSites) {
    if (!local.enabled) continue
    push(applyOverride(localToSite(local), overrides))
    const apiKey = normalizeSourceUrl(local.api)
    if (apiKey) manualApis.add(apiKey)
  }

  for (const source of sources) {
    if (!source.enabled) continue
    const parsed = getParsed(source.id)
    if (!parsed) continue
    for (const site of parsed.sites) {
      // 只在「手动源 vs 配置源」之间按接口地址去重；配置源内部同 API 的
      // 不同站点（例如同一搜索页配两套规则）仍然都要保留。
      if (manualApis.has(normalizeSourceUrl(site.api))) continue
      push(applyOverride({ ...site, origin: 'config' }, overrides))
    }
  }

  out.sort((a, b) => {
    const oa = overrides[a.key]?.order ?? Number.MAX_SAFE_INTEGER
    const ob = overrides[b.key]?.order ?? Number.MAX_SAFE_INTEGER
    if (oa !== ob) return oa - ob
    return a.name.localeCompare(b.name, 'zh-Hans-CN')
  })
  return out
}

function applyOverride(base: Site, overrides: OverrideMap): Site {
  const ov = overrides[base.key]
  if (!ov) return base
  const merged: Site = { ...base }
  if (ov.enabled !== undefined) merged.enabled = ov.enabled
  if (ov.name) merged.name = ov.name
  if (ov.timeout) merged.timeout = ov.timeout
  if (ov.header) merged.header = { ...merged.header, ...ov.header }
  if (ov.health !== undefined) merged.health = ov.health
  return merged
}

export async function getSite(key: string): Promise<Site | undefined> {
  return (await getSites()).find((s) => s.key === key)
}

export async function setSiteEnabled(key: string, enabled: boolean): Promise<void> {
  const map = await readOverrides()
  map[key] = { ...(map[key] ?? {}), enabled }
  await writeOverrides(map)
  if (enabled) {
    const site = (await getSites()).find((item) => item.key === key)
    if (site?.spiderClass) void prewarmSpider(site).catch(() => undefined)
  }
}

export async function setSitesEnabled(keys: string[], enabled: boolean): Promise<void> {
  const map = await readOverrides()
  for (const key of keys) map[key] = { ...(map[key] ?? {}), enabled }
  await writeOverrides(map)
}

export async function setSiteOrder(key: string, order: number): Promise<void> {
  const map = await readOverrides()
  map[key] = { ...(map[key] ?? {}), order }
  await writeOverrides(map)
}

/** 连通性测试：发一次最小请求，能解析出结果就算通 */
export async function testSite(key: string): Promise<TestResult> {
  const site = await getSite(key)
  if (!site) return { ok: false, ms: 0, message: '站点不存在' }
  if (site.unsupported) return { ok: false, ms: 0, message: site.unsupported }
  if (!site.api) return { ok: false, ms: 0, message: '站点未配置接口地址' }

  const settings = await getSettings()
  const timeout = site.timeout ?? settings.timeout
  const start = Date.now()

  try {
    if (site.spiderClass) {
      const data = await spiderCall<{ class?: unknown[]; list?: unknown[] }>(
        site,
        'home',
        { filter: true },
        60_000
      )
      const videos = Array.isArray(data?.list) ? data.list.length : 0
      const categories = Array.isArray(data?.class) ? data.class.length : 0
      if (!videos && !categories) {
        // 不少 Spider 只有搜索、没有首页；首页为空不代表站点不可用。
        let hits = 0
        for (const probe of ['电影', '庆余年']) {
          const list = await spiderSearch(site, probe, 3)
          hits = list.length
          if (hits) break
        }
        if (!hits) {
          return {
            ok: false,
            ms: Date.now() - start,
            message: 'Spider 连通，但首页和搜索都没有返回内容',
            category: 'empty_search'
          }
        }
        await saveHealth(key, Date.now() - start)
        return {
          ok: true,
          ms: Date.now() - start,
          message: `Spider 正常，首页为空，搜索命中 ${hits} 条`
        }
      }
      await saveHealth(key, Date.now() - start)
      return {
        ok: true,
        ms: Date.now() - start,
        message: `Spider 正常，${categories} 个分类 / 首页 ${videos} 条`
      }
    }

    if (site.type === 3 || site.type === 4) {
      const payload = await requestCms(site, { ac: 'detail', pg: 1 })
      if (!isCmsOk(payload)) {
        return { ok: false, ms: Date.now() - start, message: `接口返回异常：${(payload as any).msg ?? 'code 非成功'}` }
      }
      const list = (payload as any).list
      const count = Array.isArray(list) ? list.length : 0
      if (!count) return { ok: false, ms: Date.now() - start, message: '接口连通但未返回数据' }
      const sample = String((list[0] as any)?.vod_name ?? '')
      await saveHealth(key, Date.now() - start)
      return { ok: true, ms: Date.now() - start, message: `正常，返回 ${count} 条`, sample }
    }

    if (site.type === 1) {
      // 网页规则站：请求搜索地址，再用 rules.list 选择器验证页面结构是否还认得出来。
      // 注意 rules.list 是 CSS 选择器而不是 URL，不能拿它去发请求。
      const selector = site.rules?.list?.trim()
      const template = (site.rules?.search || site.api).trim()
      const target = template.replace(/\{(wd|kw|key|keyword)\}/gi, encodeURIComponent('测试'))
      if (!/^https?:\/\//i.test(target)) {
        return { ok: false, ms: 0, message: '网页规则站需要一个 http(s) 地址，当前地址不可请求' }
      }
      const header = await resolveHeader(site, target)
      const text = await fetchText(target, { header, timeout })
      const len = text.length
      if (len < 50) return { ok: false, ms: Date.now() - start, message: '页面内容过短，可能被拦截' }
      if (selector) {
        const count = cheerio.load(text)(selector).length
        if (!count) {
          return {
            ok: false,
            ms: Date.now() - start,
            message: `页面可访问，但列表选择器「${selector}」没有匹配到节点，规则可能已失效`
          }
        }
        await saveHealth(key, Date.now() - start)
        return { ok: true, ms: Date.now() - start, message: `正常，匹配到 ${count} 个列表节点` }
      }
      await saveHealth(key, Date.now() - start)
      return { ok: true, ms: Date.now() - start, message: `正常，页面 ${len} 字节` }
    }

    return { ok: false, ms: 0, message: site.unsupported ?? '该类型暂不支持测试' }
  } catch (err) {
    await saveHealth(key, -1)
    const message = (err as Error).message
    return {
      ok: false,
      ms: Date.now() - start,
      message,
      category: site.spiderClass ? classifySpiderFailure(message) : undefined
    }
  }
}

async function saveHealth(key: string, ms: number): Promise<void> {
  const map = await readOverrides()
  map[key] = { ...(map[key] ?? {}), health: ms }
  await writeOverrides(map)
}
