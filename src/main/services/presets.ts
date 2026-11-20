import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { app } from 'electron'
import type { ConfigSource, SourcePresetInfo } from '@shared/types'
import { normalizeSourceUrl } from '@shared/url'
import { addSource, listSources, syncSource } from './sources'
import { readStore, writeStore } from './store'

/** 记住「内置订阅已经送过了」，避免用户删光后重启又冒出来 */
const BUNDLED_FLAG = 'bundled-sources'
/** 用户指定的四个 TVBox 源只预置一次；用户删掉后不复活 */
const REMOTE_BUNDLED_FLAG = 'bundled-remote-sources'

/**
 * 内置推荐订阅。
 *
 * 打包版走 `ensureBundledSources()`：装完直接就是「带源」状态，打开就能搜。
 * 设置页里仍然把它列出来，用户可以随时删掉，删干净了也不会自己长回来。
 *
 * 点「添加」时会把配置内容落到 <数据目录>/presets/<id>.json，
 * 而不是引用仓库里的 samples/，这样仓库删了、应用换台机器也能继续用。
 */
export interface SourcePreset {
  id: string
  name: string
  desc: string
  /** 给界面显示用：这个订阅里有几个站点 */
  siteCount: number
  /** 给界面显示用：这个订阅里有几个直播源 */
  liveCount: number
  config: {
    format: string
    name: string
    note: string
    sites: Record<string, unknown>[]
    lives: Record<string, unknown>[]
    tvboxDesktop?: Record<string, unknown>
  }
}

const PUBLIC_CMS: SourcePreset = {
  id: 'public-cms',
  name: '公开可用源（苹果CMS 采集站）',
  desc: '实测存活的 5 个公开采集接口 + 2 个公开直播源。这类源随时可能失效，失效后在「影视源」「电视直播」页一看便知。',
  siteCount: 5,
  liveCount: 2,
  config: {
    format: 'tvbox-desktop/1',
    name: '公开可用源（苹果CMS 采集站）',
    note: '实测存活的公开采集接口，全部是标准苹果CMS v10 的 /api.php/provide/vod。判断标准有三条：列表接口有数据、带 wd 的搜索有结果、详情能解析出剧集地址。直播源一 txt 一 m3u，用来覆盖两种解析格式。',
    sites: [
      {
        key: 'ffzy',
        name: '非凡资源',
        type: 'cms',
        api: 'https://api.ffzyapi.com/api.php/provide/vod',
        searchable: 1
      },
      {
        key: 'lzi',
        name: '量子资源',
        type: 'cms',
        api: 'https://cj.lziapi.com/api.php/provide/vod',
        searchable: 1
      },
      {
        key: 'dyttzy',
        name: '电影天堂资源',
        type: 'cms',
        api: 'https://caiji.dyttzyapi.com/api.php/provide/vod',
        searchable: 1
      },
      {
        key: 'guangsu',
        name: '光速资源',
        type: 'cms',
        api: 'https://api.guangsuapi.com/api.php/provide/vod',
        searchable: 1
      },
      {
        key: 'subo',
        name: '素博资源',
        type: 'cms',
        api: 'https://subocaiji.com/api.php/provide/vod',
        searchable: 1
      }
    ],
    // 只留 IPv4 那一条。实测 Gather.m3u 的 123 条线路里 108 条（88%）拉不起来，
    // 而且它和 iptv4.txt 一个重名的频道都没有（补不上线路），留着只会让频道列表
    // 里多出一堆点开就失败的卡。
    lives: [
      {
        name: '公开直播源·IPv4（txt）',
        type: 0,
        url: 'https://live.zbds.top/tv/iptv4.txt'
      }
    ],
    // 首页内容位。写在这里的行优先于「每个源一条最近更新」的默认行为，
    // 让首启就有几行带分类的内容，而不是五条一模一样的「最近更新」。
    // typeId 用的是苹果CMS v10 的默认分类 id（6=动作片、13=国产剧、42=日本动漫），
    // 这五条都在真实源上逐条验证过 20 条有内容。
    tvboxDesktop: {
      home: {
        sections: [
          { title: '电影 · 最近更新', site: 'dyttzy' },
          { title: '电影 · 动作片', site: 'ffzy', typeId: 6 },
          { title: '电视剧 · 国产剧', site: 'lzi', typeId: 13 },
          { title: '动漫 · 日本', site: 'guangsu', typeId: 42 },
          { title: '综艺 · 最近更新', site: 'subo' }
        ]
      }
    }
  }
}

export const SOURCE_PRESETS: SourcePreset[] = [PUBLIC_CMS]

/**
 * 用户指定的常用 TVBox 配置。它们不是内联 JSON：
 * 配置服务器按 UA 返回不同内容，且包含图片/加密包装，统一在同步时由
 * configPayload.ts 解包。点播站点基本都是 jar/py/js Spider：jar 由内置
 * JVM 宿主运行（默认关闭、按需启用），py/js 仍会明确标注不支持。
 */
export interface RemotePreset {
  id: string
  name: string
  desc: string
  url: string
}

export const REMOTE_PRESETS: RemotePreset[] = [
  {
    id: 'fm-config',
    name: '肥猫接口',
    desc: '加密 TVBox 配置（自动解密）。点播依赖 jar Spider，可在「影视源」页启用后运行。',
    url: 'http://肥猫.net'
  },
  {
    id: 'fty-config',
    name: '饭太硬接口',
    desc: '图片伪装的 TVBox 配置（自动解包）。jar Spider 可启用运行，JS Spider 仍不支持。',
    url: 'http://www.饭太硬.net/tv'
  },
  {
    id: 'moyuer-config',
    name: '摸鱼接口',
    desc: '标准 TVBox 配置。点播依赖 jar Spider，可在「影视源」页启用后运行。',
    url: 'http://我不是.摸鱼儿.cc/'
  },
  {
    id: 'xiaosa-config',
    name: '潇洒接口',
    desc: '加密 TVBox 配置（自动解密）。jar Spider 可启用运行，py Spider 仍不支持。',
    url: 'https://9877.kstore.space/sun.json'
  }
]

export function listPresets(): SourcePresetInfo[] {
  return [
    ...SOURCE_PRESETS.map(({ id, name, desc, siteCount, liveCount }) => ({
      id,
      name,
      desc,
      siteCount,
      liveCount
    })),
    ...REMOTE_PRESETS.map(({ id, name, desc }) => ({
      id,
      name,
      desc,
      siteCount: 0,
      liveCount: 0,
      remote: true
    }))
  ]
}

/** 把预设落成数据目录里的一个本地配置文件，再注册成配置源 */
export async function addPreset(id: string): Promise<ConfigSource> {
  const remote = REMOTE_PRESETS.find((p) => p.id === id)
  if (remote) {
    const existing = await listSources()
    if (existing.some((s) => normalizeSourceUrl(s.url) === normalizeSourceUrl(remote.url))) {
      throw new Error('这个订阅已经添加过了')
    }
    return addSource({ name: remote.name, url: remote.url })
  }

  const preset = SOURCE_PRESETS.find((p) => p.id === id)
  if (!preset) throw new Error('没有这个内置订阅')

  const dir = join(app.getPath('userData'), 'presets')
  mkdirSync(dir, { recursive: true })
  const file = join(dir, `${preset.id}.json`)
  writeFileSync(file, JSON.stringify(preset.config, null, 2), 'utf8')

  const existing = await listSources()
  const already = existing.find((s) => s.url === file)
  if (already) throw new Error('这个内置订阅已经添加过了')
  return addSource({ name: preset.name, url: file, kind: 'file' })
}

/**
 * 首启登记四个 TVBox 配置源。只写源记录，不拉网络：
 * 这样窗口能立刻显示它们，不会被这几个源的长超时拖住启动。
 */
export async function ensureRemotePresetRecords(): Promise<void> {
  if (await readStore<boolean>(REMOTE_BUNDLED_FLAG, false)) return
  for (const preset of REMOTE_PRESETS) {
    try {
      await addPreset(preset.id)
    } catch (err) {
      const message = (err as Error).message
      if (!message.includes('已经添加过')) {
        console.warn(`[presets] 预置源 ${preset.name} 登记失败：`, err)
      }
    }
  }
  await writeStore(REMOTE_BUNDLED_FLAG, true)
}

/** 窗口起来后后台同步四个 TVBox 源；失败只记日志，不影响启动 */
export async function syncRemotePresets(): Promise<void> {
  const sources = await listSources()
  for (const preset of REMOTE_PRESETS) {
    const source = sources.find(
      (item) => normalizeSourceUrl(item.url) === normalizeSourceUrl(preset.url)
    )
    if (!source || source.lastSync) continue
    try {
      await syncSource(source.id)
    } catch (err) {
      console.warn(`[presets] 预置源 ${preset.name} 后台同步失败：`, err)
    }
  }
}

/**
 * 首启把内置订阅装好，让打包版「装完即用」。
 *
 * 只在一个配置源都没有的时候执行 —— 用户自己删光了就不该再冒出来。
 * 装上之后立刻同步一次，这样首屏的「影视源」和「电视直播」就有数据，
 * 不用等用户手动点「同步配置」。
 *
 * 只带**订阅源**，不带设置：API Key、外部播放器路径、收藏与观看历史
 * 都属于使用者的个人数据，绝不能跟着安装包发出去。
 *
 * 用一个标志位记住「已经送过了」：用户之后自己把源删光、重启，不会又长回来。
 */
export async function ensureBundledSources(): Promise<boolean> {
  if (SOURCE_PRESETS.length === 0) return false
  // 端到端自检使用独立数据目录时不应再注入内置订阅，否则会与桩站点抢数据。
  if (process.env['CRAYON_DISABLE_BUNDLED_SOURCES'] === '1') return false
  if (await readStore<boolean>(BUNDLED_FLAG, false)) return false
  const existing = await listSources()
  if (existing.length) {
    await writeStore(BUNDLED_FLAG, true)
    return false
  }
  try {
    const source = await addPreset(SOURCE_PRESETS[0].id)
    await writeStore(BUNDLED_FLAG, true)
    await syncSource(source.id)
    return true
  } catch (err) {
    // 写不进去就退回首启引导页，用户仍然可以手动添加
    console.error('[presets] 首启写入内置订阅失败：', err)
    return false
  }
}

