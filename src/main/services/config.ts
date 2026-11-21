import { promises as fs } from 'node:fs'
import type {
  ConfigSource,
  HeaderRule,
  HtmlRules,
  LiveSource,
  ParseItem,
  ParsedConfig,
  Site,
  SiteType
} from '@shared/types'
import { loadConfigPayload } from './configPayload'
import { resolveSourceAddress } from '@shared/url'

/**
 * 配置解析层：同时吃 TVBox 兼容方言与本项目简洁方言。
 * 设计原则是「只做覆盖不做新增语义」——扩展字段对安卓 TVBox 无副作用。
 */

const STRING_TYPE_MAP: Record<string, SiteType> = {
  cms: 3,
  maccms: 3,
  xml: 3,
  apple: 3,
  apples: 3,
  '苹果cms': 3,
  api: 4,
  json: 4,
  html: 1,
  web: 1,
  rule: 1,
  rules: 1,
  page: 1,
  spider: 0,
  jar: 0,
  aggregate: 0,
  other: 2
}

/** 稳定短哈希，用于在没有 key 时生成站点标识 */
function shortHash(input: string): string {
  let h = 0x811c9dc5
  for (let i = 0; i < input.length; i++) {
    h ^= input.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return (h >>> 0).toString(36)
}

function normalizeApi(raw: string, baseUrl?: string): string {
  const api = (raw ?? '').trim()
  if (!api) return ''
  if (/^https?:\/\//i.test(api)) return api
  if (/^(proxy|file|assets|clan|to)`?:\/\//i.test(api)) return api
  // 形如 cms1.example.com/api.php/provide/vod/ 的裸域名
  if (/^[\w.-]+\.[a-z]{2,}(?::\d+)?(?:[/?#]|$)/i.test(api) && !/^[^/]+\.(php|json|js|py)(?:[/?#]|$)/i.test(api)) return 'https://' + api
  if (!/^[a-z][\w+.-]*:/i.test(api) && (/^\.{0,2}\//.test(api) || /\.(php|json|js|py)(?:[/?]|$)/i.test(api))) {
    return resolveSourceAddress(api, baseUrl)
  }
  return api
}

function normalizeType(raw: unknown, api: string, rules?: HtmlRules): SiteType {
  if (typeof raw === 'number' && [0, 1, 2, 3, 4].includes(raw)) return raw as SiteType
  if (typeof raw === 'string') {
    const key = raw.trim().toLowerCase()
    if (key) {
      const mapped = STRING_TYPE_MAP[key]
      if (mapped !== undefined) return mapped
      const asNum = Number(key)
      if (Number.isInteger(asNum) && asNum >= 0 && asNum <= 4) return asNum as SiteType
      // 配置里明确写了类型但我们不认识：如实标成「未知」，
      // 不要偷偷降级成 type=4（CMS 变体），否则 UI 会给出错误的可用性承诺。
      return 2
    }
  }
  // 完全没有类型信息时才按内容猜测
  if (rules && (rules.list || rules.search)) return 1
  if (/provide\/vod|api\.php|\/vod\//i.test(api)) return 3
  if (api) return 4
  return 2
}

function flag(raw: unknown, fallback = false): boolean {
  if (raw === undefined || raw === null) return fallback
  if (typeof raw === 'boolean') return raw
  const n = Number(raw)
  if (!Number.isNaN(n)) return n !== 0
  const s = String(raw).toLowerCase()
  return s === 'true' || s === 'yes' || s === '1'
}

function normalizeRules(raw: unknown): HtmlRules | undefined {
  if (!raw || typeof raw !== 'object') return undefined
  const r = raw as Record<string, unknown>
  const pick = (...keys: string[]): string | undefined => {
    for (const k of keys) {
      const v = r[k]
      if (typeof v === 'string' && v.trim()) return v.trim()
    }
    return undefined
  }
  const rules: HtmlRules = {
    list: pick('list', 'vodList', 'listRule'),
    name: pick('name', 'vodName', 'title'),
    link: pick('link', 'vodLink', 'href'),
    pic: pick('pic', 'vodPic', 'image'),
    remarks: pick('remarks', 'vodRemarks', 'note'),
    search: pick('search', 'searchUrl'),
    detailName: pick('detailName'),
    detailContent: pick('detailContent', 'content'),
    playList: pick('playList', 'playFrom'),
    playItem: pick('playItem', 'playUrl')
  }
  const cleaned = Object.fromEntries(
    Object.entries(rules).filter(([, v]) => typeof v === 'string' && v)
  ) as HtmlRules
  return Object.keys(cleaned).length ? cleaned : undefined
}

export interface SiteNormalizeContext {
  sourceId: string
  sourceName: string
  baseUrl?: string
  /** 配置顶层的 spider 地址；TVBox 的 csp_* 站点共用这一份 jar */
  spider?: string
  defaults?: { timeout?: number; header?: Record<string, string> }
}

export function normalizeSite(input: unknown, ctx: SiteNormalizeContext): Site | null {
  if (!input || typeof input !== 'object') return null
  const raw = input as Record<string, any>
  const name = String(raw.name ?? raw.title ?? '').trim()
  const api = normalizeApi(String(raw.api ?? raw.url ?? raw.ext?.api ?? ''), ctx.baseUrl)
  if (!name && !api) return null

  const rules = normalizeRules(raw.rules ?? raw.ext?.rules)
  let type = normalizeType(raw.type, api, rules)
  // TVBox 的 HTTP 0/1 分别是 XML/JSON 采集；本项目的 type=1 另用于带规则的网页站。
  const numericType = typeof raw.type === 'number' || (typeof raw.type === 'string' && /^[01]$/.test(raw.type.trim()))
  if (numericType && /^https?:\/\//i.test(api) && (type === 0 || (type === 1 && !rules))) type = 3
  const key = String(raw.key ?? raw.id ?? `csp_${shortHash(`${name}|${api}`)}`).trim()

  const site: Site = {
    key,
    name: name || key,
    type,
    api,
    ext: typeof raw.ext === 'string'
      ? (/^\.{0,2}\//.test(raw.ext) ? resolveSourceAddress(raw.ext, ctx.baseUrl) : raw.ext)
      : raw.ext ? JSON.stringify(raw.ext) : undefined,
    jar: typeof raw.jar === 'string' ? resolveSourceAddress(raw.jar, ctx.baseUrl) : ctx.spider,
    searchable: flag(raw.searchable, true),
    quickSearch: flag(raw.quickSearch, flag(raw.searchable, true)),
    filterable: flag(raw.filterable, type === 3 || type === 4),
    playerType: Number(raw.playerType ?? 1) || 1,
    categories: Array.isArray(raw.categories) ? raw.categories.map(String) : undefined,
    style:
      raw.style && typeof raw.style === 'object'
        ? { type: raw.style.type, ratio: Number(raw.style.ratio) || undefined }
        : undefined,
    header: {
      ...(ctx.defaults?.header ?? {}),
      ...(raw.header && typeof raw.header === 'object' ? raw.header : {})
    },
    timeout: Number(raw.timeout) || ctx.defaults?.timeout,
    sourceId: ctx.sourceId,
    sourceName: ctx.sourceName,
    enabled: flag(raw.enabled, true),
    rules
  }
  if (!Object.keys(site.header ?? {}).length) delete site.header

  // 桌面端不支持的能力，显式标注原因，UI 上灰显而不是静默丢弃
  if (!site.api || /^(proxy|assets|file|clan|to):/i.test(site.api)) {
    site.unsupported = '该站点依赖内置 Spider 或本地资源，桌面端无法直接请求'
    site.searchable = false
    site.quickSearch = false
  } else if (/^csp_/i.test(site.api)) {
    // csp_* 是 CatVod/TVBox 的 Spider 类名约定。运行层由 spider-host 提供，
    // 这里只记录实现类，不再把整站判死。
    site.spiderClass = `com.github.catvod.spider.${site.api.slice(4)}`
    if (!site.jar) {
      site.unsupported = '该站点依赖 Spider，但配置没有提供 spider 文件'
      site.searchable = false
      site.quickSearch = false
    }
  } else if (/\.py(?:\?|$)/i.test(site.api)) {
    site.unsupported = '该站点依赖 Python Spider（.py），当前版本未内置 Python 运行时'
    site.searchable = false
    site.quickSearch = false
  } else if (/\.js(?:\?|$)|drpy/i.test(site.api) || /^\.{1,2}\//.test(site.api)) {
    site.unsupported = '该站点依赖 JS Spider（drpy），当前版本未内置 JS 运行时'
    site.searchable = false
    site.quickSearch = false
  } else if (type === 0) {
    site.unsupported = '该站点需要配套 Spider（jar/dex），当前配置未提供可运行文件'
    site.searchable = false
    site.quickSearch = false
  } else if (type === 2) {
    site.unsupported = '未知站点类型，暂不支持'
    site.searchable = false
    site.quickSearch = false
  }
  // Spider 站点首次接入时默认关闭：一个配置里可能有几十个 csp_* 站点，
  // 全量并发会把首次搜索变成几十个 JVM 实例同时初始化。由用户在「影视源」
  // 页确认后打开；配置里显式写入 enabled=true 的已有选择仍然保留。
  if (site.spiderClass && raw.enabled === undefined) site.enabled = false
  return site
}

function normalizeParses(raw: unknown): ParseItem[] {
  if (!Array.isArray(raw)) return []
  return raw
    .map((item): ParseItem | null => {
      if (!item || typeof item !== 'object') return null
      const p = item as Record<string, any>
      const url = String(p.url ?? '').trim()
      if (!url) return null
      return {
        name: String(p.name ?? '未命名解析'),
        type: Number(p.type ?? 1) || 1,
        url,
        ext: p.ext
      }
    })
    .filter((x): x is ParseItem => x !== null)
}

function normalizeLives(raw: unknown): LiveSource[] {
  if (!Array.isArray(raw)) return []
  const out: LiveSource[] = []
  for (const item of raw) {
    if (typeof item === 'string') {
      out.push({ name: '直播源', type: 0, url: item })
      continue
    }
    if (!item || typeof item !== 'object') continue
    const l = item as Record<string, any>
    const url = String(l.url ?? '').trim()
    if (!url || /^(proxy|assets|clan|file|to):/i.test(url)) continue
    out.push({
      name: String(l.name ?? '直播源'),
      type: l.type ?? 0,
      url,
      epg: l.epg ? String(l.epg) : undefined
    })
  }
  return out
}

function normalizeHeaderRules(raw: unknown): HeaderRule[] {
  if (!Array.isArray(raw)) return []
  return raw
    .map((item): HeaderRule | null => {
      if (!item || typeof item !== 'object') return null
      const h = item as Record<string, any>
      if (!h.host || !h.header) return null
      return { host: String(h.host), header: h.header as Record<string, string> }
    })
    .filter((x): x is HeaderRule => x !== null)
}

/** 把任意一份配置 JSON 归一化成内部结构 */
export function parseConfig(rawInput: unknown, ctx: SiteNormalizeContext): ParsedConfig {
  const raw = (rawInput ?? {}) as Record<string, any>
  const ext = (raw.tvboxDesktop ?? {}) as Record<string, any>
  const settings = (raw.settings ?? {}) as Record<string, any>
  const spider = typeof raw.spider === 'string' && raw.spider.trim()
    ? resolveSourceAddress(raw.spider.trim(), ctx.baseUrl) : ctx.spider

  const defaults = {
    timeout: Number(ext.timeout ?? settings.timeout ?? ctx.defaults?.timeout) || undefined,
    header: {
      ...(ctx.defaults?.header ?? {}),
      ...(raw.header && typeof raw.header === 'object' ? raw.header : {})
    }
  }

  const sitesRaw = Array.isArray(raw.sites) ? raw.sites : []
  const seen = new Set<string>()
  const sites: Site[] = []
  for (const item of sitesRaw) {
    const site = normalizeSite(item, { ...ctx, defaults, spider })
    if (!site) continue
    if (seen.has(site.key)) continue
    seen.add(site.key)
    sites.push(site)
  }

  return {
    name: String(raw.name ?? ext.name ?? ctx.sourceName ?? '未命名配置'),
    notice: raw.notice ? String(raw.notice) : undefined,
    sites,
    parses: normalizeParses(raw.parses).map((item) => ({ ...item, url: resolveSourceAddress(item.url, ctx.baseUrl) })),
    lives: normalizeLives(raw.lives).map((item) => ({ ...item, url: resolveSourceAddress(item.url, ctx.baseUrl) })),
    flags: Array.isArray(raw.flags) ? raw.flags.map(String) : [],
    headers: normalizeHeaderRules(raw.headers ?? settings.headers),
    theme: (raw.theme ?? ext.theme) as ParsedConfig['theme'],
    externalPlayer: (raw.externalPlayer ?? ext.externalPlayer) as ParsedConfig['externalPlayer'],
    home: (raw.home ?? ext.home) as ParsedConfig['home'],
    raw: rawInput
  }
}

/** 拉取（或读取本地）一份配置源，返回归一化结果 */
export async function loadConfigSource(
  source: Pick<ConfigSource, 'id' | 'name' | 'url' | 'kind'>,
  timeout = 20
): Promise<ParsedConfig> {
  let json: unknown
  if (source.kind === 'file') {
    const text = await fs.readFile(source.url, 'utf-8')
    json = JSON.parse(text.replace(/^\uFEFF/, ''))
  } else {
    json = (await loadConfigPayload(source.url, timeout)).raw
  }
  return parseConfig(json, { sourceId: source.id, sourceName: source.name, baseUrl: source.url, defaults: { timeout } })
}
