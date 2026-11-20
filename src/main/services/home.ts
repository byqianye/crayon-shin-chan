import type { HomeSection, Site } from '@shared/types'
import { asArray, isCmsOk } from './cms'
import { requestCms } from './cmsApi'
import { cmsItemToVod } from './search'
import { getSettings } from './settings'
import { getSites } from './sites'
import { getParsed, listSources } from './sources'
import { spiderCall, spiderItemToVod } from './spider'

/** 每个内容位最多放多少条。首页是横向滑动的，太多没必要，也省流量 */
const LIMIT = 24
/** 首页内容位缓存多久。首页是每次启动都看的第一个页面，不缓存等于每次启动都打一遍所有源 */
const TTL = 10 * 60 * 1000
/**
 * 失败的那几条只缓存 1 分钟。
 *
 * 「fetch failed」很多时候是一次网络的抖动（实测素博第一次拉就失败、紧接着重试就好了），
 * 按 10 分钟缓存下来会让用户盯着一条错误的横条看很久。
 */
const ERROR_TTL = 60 * 1000

interface Plan {
  key: string
  title: string
  origin: 'config' | 'default'
  site: Site
  typeId?: string
}

/**
 * 从配置源的 `home.sections` 里读内容位。
 *
 * `site` 这一项各家配置写得不一样：有的写站点 key，有的写中文名。
 * 所以按「key 精确 → 名字精确 → 名字互相包含」三级去找，都找不到就跳过。
 */
function matchSite(sites: Site[], ref: string): Site | undefined {
  const want = ref.trim()
  if (!want) return undefined
  const lower = want.toLowerCase()
  return (
    sites.find((s) => s.key === want) ??
    sites.find((s) => s.name === want) ??
    sites.find((s) => s.key.toLowerCase() === lower || s.name.toLowerCase() === lower) ??
    sites.find((s) => s.name.includes(want) || want.includes(s.name))
  )
}

/** 苹果CMS 站点走 HTTP；Spider 站点走 JVM。网页规则站的「最近更新」暂时不做。 */
function isContentSite(site: Site): boolean {
  return (
    site.enabled &&
    !site.unsupported &&
    (Boolean(site.spiderClass) || ((site.type === 3 || site.type === 4) && Boolean(site.api)))
  )
}

function isCms(site: Site): boolean {
  return (
    (site.type === 3 || site.type === 4) &&
    site.enabled &&
    !site.unsupported &&
    !site.spiderClass &&
    !!site.api
  )
}

async function buildPlan(): Promise<{ plans: Plan[]; missing: HomeSection[] }> {
  const sites = (await getSites()).filter((s) => s.enabled)
  const sources = await listSources()
  const plans: Plan[] = []
  const missing: HomeSection[] = []
  const seen = new Set<string>()

  for (const source of sources) {
    if (!source.enabled) continue
    const sections = getParsed(source.id)?.home?.sections ?? []
    for (const raw of sections) {
      const title = String(raw?.title ?? '').trim()
      const ref = String(raw?.site ?? '').trim()
      if (!title && !ref) continue
      const site = matchSite(sites, ref)
      if (!site) {
        missing.push({
          key: `missing:${source.id}:${ref}`,
          title: title || ref,
          origin: 'config',
          items: [],
          error: `配置里写的影视源「${ref}」不在当前已启用的源里`
        })
        continue
      }
      if (!isContentSite(site)) {
        missing.push({
          key: `unsupported:${source.id}:${site.key}`,
          title: title || site.name,
          origin: 'config',
          siteKey: site.key,
          siteName: site.name,
          items: [],
          error: `${site.name} 不是可用的苹果CMS / Spider 源，首页内容位暂不支持`
        })
        continue
      }
      const typeId = raw?.typeId === undefined || raw?.typeId === '' ? undefined : String(raw.typeId)
      const key = `${site.key}::${typeId ?? 'latest'}`
      if (seen.has(key)) continue
      seen.add(key)
      plans.push({
        key,
        title: title || (typeId ? `${site.name} · ${typeId}` : `${site.name} · 最近更新`),
        origin: 'config',
        site,
        typeId
      })
    }
  }

  // 一条内容位都没配时给默认值：每个可用源一条「最近更新」。
  // 这样第一次用的用户打开首页就有东西看，而不是一片空白。
  if (!plans.length) {
    for (const site of sites) {
      if (!isCms(site)) continue
      plans.push({
        key: `${site.key}::latest`,
        title: `${site.name} · 最近更新`,
        origin: 'default',
        site
      })
    }
    // 只有 Spider 源的配置也给一屏；限制条数，避免一次拉起几十个 JVM 站点。
    if (!plans.length) {
      for (const site of sites.filter((s) => s.spiderClass && isContentSite(s)).slice(0, 8)) {
        plans.push({
          key: `${site.key}::latest`,
          title: `${site.name} · 最近更新`,
          origin: 'default',
          site
        })
      }
    }
  }

  return { plans, missing }
}

async function loadSection(plan: Plan): Promise<HomeSection> {
  const base = {
    key: plan.key,
    title: plan.title,
    origin: plan.origin,
    siteKey: plan.site.key,
    siteName: plan.site.name
  }
  try {
    if (plan.site.spiderClass) {
      const payload = plan.typeId
        ? await spiderCall<{ list?: unknown[] }>(
            plan.site,
            'category',
            { tid: plan.typeId, pg: '1', filter: true, extend: {} },
            120_000
          )
        : await spiderCall<{ list?: unknown[] }>(plan.site, 'home', { filter: true }, 120_000)
      const items = asArray<Record<string, unknown>>(payload?.list)
        .slice(0, LIMIT)
        .map((item) => spiderItemToVod(item, plan.site))
      if (!items.length) return { ...base, items: [], error: '这一个内容位没有返回数据' }
      return { ...base, items }
    }
    // ac=detail 会连 vod_play_url 一起带回来，点进详情就不用再请求一次
    const payload = await requestCms(plan.site, { ac: 'detail', t: plan.typeId, pg: 1 })
    if (!isCmsOk(payload)) {
      const msg = String((payload as Record<string, unknown>)?.msg ?? '') || 'code 非成功'
      return { ...base, items: [], error: `接口返回异常：${msg}` }
    }
    const items = asArray<Record<string, unknown>>((payload as Record<string, unknown>)?.list)
      .slice(0, LIMIT)
      .map((item) => cmsItemToVod(item, plan.site))
    if (!items.length) return { ...base, items: [], error: '这一个内容位没有返回数据' }
    return { ...base, items }
  } catch (err) {
    return { ...base, items: [], error: (err as Error).message }
  }
}

let cache: { sig: string; at: number; sections: HomeSection[] } | null = null
let homeGeneration = 0
let loading: { sig: string; gen: number; task: Promise<HomeSection[]> } | null = null

/**
 * 首页内容位。配置里有 `home.sections` 就按配置来，没有就每个可用源给一条「最近更新」。
 *
 * 拉取是**失败不抛**的：某个源挂了只让那一条内容位自己显示错误，
 * 不能让整个首页白屏。
 */
export async function listHomeSections(force = false): Promise<HomeSection[]> {
  const gen = homeGeneration
  const { plans, missing } = await buildPlan()
  if (gen !== homeGeneration) return listHomeSections(force)
  const sig = JSON.stringify(plans)

  if (!force && cache && cache.sig === sig) {
    const age = Date.now() - cache.at
    // 上一次有拉失败的内容位时只用短 TTL，让抖动过的源很快有机会翻身
    const limit = cache.sections.some((s) => s.error) ? ERROR_TTL : TTL
    if (age < limit) return [...cache.sections, ...missing]
  }

  if (loading?.sig === sig && loading.gen === gen) return [...await loading.task, ...missing]
  const task = loadPlans(plans, sig, gen)
  loading = { sig, gen, task }
  try { return [...await task, ...missing] }
  finally { if (loading?.task === task) loading = null }
}

async function loadPlans(plans: Plan[], sig: string, gen: number): Promise<HomeSection[]> {
  const settings = await getSettings()
  const concurrency = Math.max(1, Math.min(32, settings.concurrency ?? 8))
  const sections: HomeSection[] = new Array(plans.length)
  let cursor = 0
  const worker = async (): Promise<void> => {
    for (;;) {
      const index = cursor++
      if (index >= plans.length) return
      sections[index] = await loadSection(plans[index])
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, plans.length) }, () => worker()))

  const filled = sections.filter(Boolean)
  if (gen === homeGeneration) cache = { sig, at: Date.now(), sections: filled }
  return filled
}

/** 站点列表变了（换源、启用开关）就得让缓存失效 */
export function clearHomeCache(): void {
  homeGeneration++
  loading = null
  cache = null
}
