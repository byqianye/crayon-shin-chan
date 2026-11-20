/**
 * 主进程 spider.ts 的集成冒烟：
 * 用 esbuild 把服务 bundle 成 CJS，再用 electron 桩跑一遍
 * search → detail → player → ping → stop。
 *
 * 需要：
 *   SPIDER_JAVA=<java.exe>
 *   SPIDER_JAR=<spider zip/jar 路径>
 *   SPIDER_DEX_TOOLS=<dex-tools 目录，默认 spider-host/dex-tools/dex-tools-v2.4>
 */
import { existsSync } from 'node:fs'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

const root = resolve(import.meta.dirname, '..')
const java = process.env.SPIDER_JAVA || (process.env.JAVA_HOME ? join(process.env.JAVA_HOME, 'bin', 'java.exe') : '')
const jar = process.env.SPIDER_JAR || ''
const dexTools = process.env.SPIDER_DEX_TOOLS || join(root, 'spider-host', 'dex-tools', 'dex-tools-v2.4')
const esbuild = process.env.ESBUILD_EXE || join(
  root,
  'node_modules',
  '.pnpm',
  '@esbuild+win32-x64@0.25.12',
  'node_modules',
  '@esbuild',
  'win32-x64',
  'esbuild.exe'
)

if (!java || !existsSync(java)) {
  console.error('[skip] 请设置 SPIDER_JAVA 指向 java.exe')
  process.exit(0)
}
if (!jar || !existsSync(jar)) {
  console.error('[skip] 请设置 SPIDER_JAR 指向一个 spider zip/jar')
  process.exit(0)
}
if (!existsSync(join(dexTools, 'lib'))) {
  console.error(`[skip] 缺少 dex-tools：${dexTools}`)
  process.exit(0)
}

const temp = await mkdtemp(join(tmpdir(), 'crayon-spider-smoke-'))
try {
  const stub = join(temp, 'electron-stub.cjs')
  await writeFile(
    stub,
    `const path = require('node:path')
module.exports = {
  app: {
    isPackaged: false,
    getAppPath: () => process.env.SPIDER_PROJECT_ROOT,
    getPath: () => process.env.SPIDER_USERDATA
  }
}
`,
    'utf8'
  )

  const bundle = join(temp, 'spider.cjs')
  const bundled = spawnSync(esbuild, [
    join(root, 'src', 'main', 'services', 'spider.ts'),
    '--bundle',
    '--platform=node',
    '--format=cjs',
    `--outfile=${bundle}`,
    `--alias:electron=${stub}`,
    `--alias:@shared=${join(root, 'src', 'shared')}`,
    `--tsconfig=${join(root, 'tsconfig.node.json')}`,
    '--log-level=error'
  ], { stdio: 'inherit' })
  if (bundled.status !== 0) throw new Error('esbuild 打包失败')

  const runner = join(temp, 'run.cjs')
  await writeFile(
    runner,
    `const spider = require(${JSON.stringify(bundle)})
const site = {
  key: process.env.SPIDER_KEY || 'jianpian',
  name: process.env.SPIDER_NAME || '荐片',
  type: 3,
  api: process.env.SPIDER_API || 'csp_Jianpian',
  ext: process.env.SPIDER_EXT || 'https://api.ztcgi.com',
  spiderClass: process.env.SPIDER_CLASS || 'com.github.catvod.spider.Jianpian',
  jar: process.env.SPIDER_JAR,
  searchable: true,
  quickSearch: true,
  filterable: true,
  playerType: 1,
  sourceId: 'smoke',
  sourceName: 'smoke',
  enabled: true
}

async function main() {
  const list = await spider.spiderSearch(site, process.env.SPIDER_KEYWORD || '庆余年', 5)
  console.log('[search]', list.length, list[0] && list[0].vod_name)
  if (!list.length) throw new Error('search returned no result')

  const payload = await spider.spiderCall(site, 'detail', { ids: [list[0].vod_id] })
  const raw = payload.list && payload.list[0]
  if (!raw) throw new Error('detail returned no item')
  if (process.env.SPIDER_DUMP) console.log('[detail-raw]', JSON.stringify(payload).slice(0, 3000))
  console.log('[detail]', raw && raw.vod_name, 'playFrom', raw && raw.vod_play_from)
  const flags = String(raw.vod_play_from || '').split('$$$').filter(Boolean)
  const group = String(raw.vod_play_url || '').split('$$$')[0] || ''
  const episode = group.split('#').filter(Boolean)[0] || ''
  const id = episode.includes('$') ? episode.slice(episode.lastIndexOf('$') + 1) : episode
  if (!flags.length || !id) throw new Error('detail returned no playable line')
  const played = await spider.spiderPlayerUrl(site, flags[0], id)
  console.log('[player]', played.ok, played.url && played.url.slice(0, 100))
  if (!played.url) throw new Error('player returned no url')
  const mem = await spider.spiderCall(site, 'ping')
  console.log('[jvm]', JSON.stringify(mem))
  await spider.stopSpiderHost()
}

main().catch(async (error) => {
  console.error('[fail]', error)
  try { await spider.stopSpiderHost() } catch {}
  process.exitCode = 1
})
`,
    'utf8'
  )

  const result = spawnSync(process.execPath, [runner], {
    env: {
      ...process.env,
      SPIDER_JAVA: java,
      SPIDER_JAR: jar,
      SPIDER_DEX_TOOLS: dexTools,
      SPIDER_HOST_ROOT: join(root, 'spider-host'),
      SPIDER_PROJECT_ROOT: root,
      SPIDER_USERDATA: join(temp, 'userdata')
    },
    stdio: 'inherit'
  })
  process.exitCode = result.status ?? 1
} finally {
  await rm(temp, { recursive: true, force: true })
}
