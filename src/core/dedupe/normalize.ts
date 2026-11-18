/**
 * 片名归一化、年份/季数提取与内容分类。
 *
 * 归一化的目的：让不同站点上的同一部影片落到同一个 key 上。
 * 采集站对同一部剧的写法千奇百怪，实际见过的有：
 *   庆余年第二季 / 庆余年 第二季 / 庆余年（第二季） / 庆余年第二季[全36集]
 *   庆余年，全世界都以为我是废物太子（庆余年之大凉太子爷）
 * 这些应该合并；而下面这些必须保持区分，否则会把不同的作品合成一部：
 *   庆余年第一季 / 庆余年第二季 / 庆余年之帝王业 / 庆余年之风起沧州
 *
 * 所以规则是「只去掉不影响作品身份的噪声」，季数、部数、副标题一律保留。
 */

/** 全角转半角（只处理 ASCII 可见区，中文标点单独在标点表里） */
function toHalfWidth(text: string): string {
  return text.replace(/[\uFF01-\uFF5E]/g, (ch) => String.fromCharCode(ch.charCodeAt(0) - 0xfee0))
}

/**
 * 会被丢掉的标点与符号。
 * 注意不要放「第」「季」「部」「集」这些能区分作品的字。
 */
const PUNCTUATION = /[\s·・:：;；\-—_~～.,，。!！?？'"“”‘’`^&*+=<>《》()（）[\]【】{}｛｝/\\|@#$%]/g

/**
 * 同一部作品的常见后缀/前缀噪声，去掉后不影响身份判定。
 * 这些词在采集站上经常被随意加上，加了就是同一部剧。
 */
const NOISE_WORDS =
  /(高清版?|超清|蓝光|完整版?|全集|全\d+集|国语版?|粤语版?|普通话版?|中字|双语|hd|bd|4k|1080p|720p|2160p)/gi

/**
 * 尾部的别名括号。采集站常把别名直接塞在片名后面，实际见过的有：
 *   庆余年，全世界都以为我是废物太子（庆余年之大凉太子爷）
 * 括号里只是同一部剧的另一个叫法，应该并到主名上。
 *
 * 但括号里若是「第二季 / 第61-81集 / 特别篇」这种分季分集信息就绝不能丢，
 * 否则会把第一季和第二季合成一部。
 */
const ALIAS_TAIL = /[（(]([^（()）]*)[)）]\s*$/
const SEASON_MARK = /第\s*[0-9一二三四五六七八九十百千]+\s*[季部集篇辑]|番外|特别篇|前传|后传|续集/

/** 去掉不影响作品身份的尾部括号别名；整名都是括号时保持原样 */
function stripAliasTail(text: string): string {
  let out = text.trim()
  for (;;) {
    const m = out.match(ALIAS_TAIL)
    if (!m || SEASON_MARK.test(m[1])) return out
    const head = out.slice(0, m.index ?? 0).trim()
    if (!head) return out
    out = head
  }
}

/** 归一化片名。空名字返回空串，调用方靠空串跳过。 */
export function normalizeName(name: string): string {
  if (!name) return ''
  return toHalfWidth(stripAliasTail(name))
    .replace(NOISE_WORDS, '')
    .replace(PUNCTUATION, '')
    .toLowerCase()
}

/**
 * 解说 / 二创类内容。用户明确要求「去除掉每个源的电影解说」。
 *
 * 这些条目在采集站上通常是几分钟的解说短片，和正片同名同封面，
 * 混在搜索结果里非常干扰，而且点进去往往只有一集。
 *
 * 两条线索都用上：
 *   · 片名，例如「庆余年[电影解说]」「XX一口气看完」
 *   · 分类，例如 vod_class/vod_type_name = 「影视解说」「电影解说」
 * 只按片名判断会漏掉那些片名干净、靠分类标记的条目。
 */
const COMMENTARY_RE = /(解说|速看|盘点|混剪|拆解|影评|剧情介绍|一口气看完|分钟看完)/

/** 片名或分类看起来是不是解说/二创内容 */
export function isCommentary(name: string, typeName?: string): boolean {
  return COMMENTARY_RE.test(name ?? '') || COMMENTARY_RE.test(typeName ?? '')
}

/**
 * 把同名影片的候选按「信息量」排序，用来挑代表条目。
 * 有海报、有备注、有评分的排前面——这样合并后的卡片看起来最完整。
 */
export function scoreCandidate(vod: {
  vod_pic?: string
  vod_remarks?: string
  vod_score?: string | number
}): number {
  let score = 0
  if (vod.vod_pic) score += 4
  if (vod.vod_remarks) score += 2
  if (vod.vod_score && Number(vod.vod_score) > 0) score += 1
  return score
}

/** 从年份字段或片名的括号里取 4 位年份 */
export function extractYear(...values: (string | number | undefined)[]): string {
  for (const value of values) {
    const text = String(value ?? '').trim()
    if (!text) continue
    // 年份字段常见 "2024" / "2024-01-01"；只认开头，避免把片名里的数字当年份
    const exact = text.match(/^((?:19|20)\d{2})(?:$|[-/.])/)
    if (exact) return exact[1]
    const inTitle = text.match(/[（(]\s*((?:19|20)\d{2})\s*[)）]/)
    if (inTitle) return inTitle[1]
    const tail = text.match(/(?:^|[\s._-])((?:19|20)\d{2})$/)
    if (tail) return tail[1]
  }
  return ''
}

const CN_DIGITS: Record<string, number> = {
  零: 0,
  一: 1,
  二: 2,
  两: 2,
  三: 3,
  四: 4,
  五: 5,
  六: 6,
  七: 7,
  八: 8,
  九: 9
}

/** 只覆盖片名里常见的 1–99 中文数字 */
function cnToNumber(text: string): string {
  if (/^\d+$/.test(text)) return String(Number(text))
  if (text === '十') return '10'
  const ten = text.match(/^([一二两三四五六七八九])?十([一二三四五六七八九])?$/)
  if (ten) {
    const high = ten[1] ? CN_DIGITS[ten[1]] : 1
    const low = ten[2] ? CN_DIGITS[ten[2]] : 0
    return String(high * 10 + low)
  }
  if (CN_DIGITS[text] !== undefined) return String(CN_DIGITS[text])
  return text
}

/**
 * 提取季/部标记。返回数字字符串，没有则空串。
 * 「第2季」「第二部」「Season 2」「S02」都归一到 "2"。
 */
export function extractSeason(name: string): string {
  const text = name ?? ''
  const cn = text.match(/第\s*([0-9一二两三四五六七八九十]+)\s*[季部]/)
  if (cn) return cnToNumber(cn[1])
  const en = text.match(/(?:season|series)\s*(\d{1,2})\b/i)
  if (en) return String(Number(en[1]))
  const short = text.match(/\bS(\d{1,2})\b/i)
  if (short) return String(Number(short[1]))
  return ''
}

export type ContentBucket = 'movie' | 'tv' | 'anime' | 'variety' | 'other'

/** 把各站五花八门的分类名归成五类，只用于去重时的硬约束 */
export function typeBucket(typeName?: string): ContentBucket {
  const text = typeName ?? ''
  if (/动漫|动画|番剧|卡通/.test(text)) return 'anime'
  if (/综艺|真人秀|脱口秀/.test(text)) return 'variety'
  if (
    /电视剧|连续剧|剧集|短剧|美剧|韩剧|日剧|港剧|台剧|国产剧|海外剧|香港剧|台湾剧|大陆剧|欧美剧/.test(
      text
    )
  ) {
    return 'tv'
  }
  if (/电影|影片|片/.test(text)) return 'movie'
  return 'other'
}

function levenshtein(a: string, b: string): number {
  if (a === b) return 0
  if (!a.length) return b.length
  if (!b.length) return a.length
  let previous = Array.from({ length: b.length + 1 }, (_v, i) => i)
  for (let i = 1; i <= a.length; i++) {
    const current = [i]
    for (let j = 1; j <= b.length; j++) {
      current[j] = Math.min(
        previous[j] + 1,
        current[j - 1] + 1,
        previous[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1)
      )
    }
    previous = current
  }
  return previous[b.length]
}

/** 归一化片名的相似度，0–1；用于兜底合并轻微写法差异 */
export function titleSimilarity(a: string, b: string): number {
  const left = normalizeName(a)
  const right = normalizeName(b)
  if (!left || !right) return 0
  if (left === right) return 1
  const longest = Math.max(left.length, right.length)
  return 1 - levenshtein(left, right) / longest
}

export interface DedupeInput {
  vod_name: string
  vod_year?: string | number
  type_name?: string
}

/**
 * 去重键：类型 + 归一化片名 + 季数 + 年份。
 * 年份缺失时留空，由合并函数用「标题完全相等」兜底，避免把不同版本误合。
 */
export function buildDedupeKey(vod: DedupeInput): string {
  return [
    typeBucket(vod.type_name),
    normalizeName(vod.vod_name),
    extractSeason(vod.vod_name),
    extractYear(vod.vod_year, vod.vod_name)
  ].join('|')
}
