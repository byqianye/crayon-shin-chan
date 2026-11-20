/**
 * Spider 宿主 PoC 冒烟脚本：
 * 启动 spider-host，依次调用 ping / home / search / detail，输出耗时与 RSS。
 *
 * 用法：
 *   node scripts/spider-smoke.mjs --java <java.exe> --jar <spider.jar> \
 *     [--class com.github.catvod.spider.Jianpian] [--ext https://api.ztcgi.com]
 */
import { spawn, spawnSync } from 'node:child_process'
import { createInterface } from 'node:readline'
import { resolve } from 'node:path'

const argv = process.argv.slice(2)
const arg = (name, fallback = '') => {
  const index = argv.indexOf(`--${name}`)
  return index >= 0 && argv[index + 1] ? argv[index + 1] : fallback
}

const java = arg('java', process.env.JAVA_HOME ? resolve(process.env.JAVA_HOME, 'bin', 'java.exe') : 'java')
const jar = resolve(arg('jar'))
const className = arg('class', 'com.github.catvod.spider.Jianpian')
const ext = arg('ext', 'https://api.ztcgi.com')
const workdir = resolve(import.meta.dirname, '..')
const libDir = resolve(workdir, 'spider-host', 'lib')
const classpath = `${resolve(workdir, 'spider-host', 'out')};${resolve(libDir, 'android-4.1.1.4.jar')};${resolve(libDir, 'json-20090211.jar')};${resolve(libDir, '*')}`

const child = spawn(java, [
  '-Xms16m',
  '-Xmx192m',
  '-XX:+UseSerialGC',
  '-XX:TieredStopAtLevel=1',
  '-XX:MaxMetaspaceSize=96m',
  '-XX:ReservedCodeCacheSize=48m',
  '-noverify',
  '-Dfile.encoding=UTF-8',
  '-cp',
  classpath,
  'host.SpiderHost',
  '--jar',
  jar,
  '--class',
  className,
  '--ext',
  ext
], { cwd: workdir, stdio: ['pipe', 'pipe', 'pipe'] })

child.stderr.on('data', (chunk) => process.stderr.write(chunk))

const pending = new Map()
let nextId = 1
let readyResolve
const ready = new Promise((resolveReady) => {
  readyResolve = resolveReady
})

createInterface({ input: child.stdout }).on('line', (line) => {
  let message
  try {
    message = JSON.parse(line)
  } catch {
    console.log('[stdout]', line)
    return
  }
  if (message.event === 'ready') {
    readyResolve(message)
    return
  }
  const waiter = pending.get(message.id)
  if (!waiter) return
  pending.delete(message.id)
  clearTimeout(waiter.timer)
  if (message.ok) waiter.resolve(message.data)
  else waiter.reject(new Error(message.error || 'spider error'))
})

function call(method, params = {}, timeoutMs = 30_000) {
  const id = nextId++
  return new Promise((resolveCall, rejectCall) => {
    const timer = setTimeout(() => {
      pending.delete(id)
      rejectCall(new Error(`${method} timeout after ${timeoutMs}ms`))
    }, timeoutMs)
    pending.set(id, { resolve: resolveCall, reject: rejectCall, timer })
    child.stdin.write(`${JSON.stringify({ id, method, params })}\n`)
  })
}

function rssMb() {
  const result = spawnSync('powershell', [
    '-NoProfile',
    '-Command',
    `(Get-Process -Id ${child.pid} -ErrorAction SilentlyContinue).WorkingSet64`
  ], { encoding: 'utf8' })
  const bytes = Number(result.stdout.trim())
  return Number.isFinite(bytes) && bytes > 0 ? Math.round(bytes / 1024 / 1024) : -1
}

async function timed(label, fn) {
  const started = performance.now()
  const data = await fn()
  const ms = Math.round(performance.now() - started)
  console.log(`[ok] ${label} ${ms}ms rss=${rssMb()}MB`)
  return data
}

try {
  const readyInfo = await ready
  console.log(`[ready] ${readyInfo.class ?? `sites=${readyInfo.sites}`} pid=${readyInfo.pid} rss=${rssMb()}MB`)

  await timed('ping', () => call('ping'))
  const home = await timed('home', () => call('home', { filter: true }))
  console.log(`[home] categories=${home?.class?.length ?? 0} videos=${home?.list?.length ?? 0}`)

  const key = arg('key', '庆余年')
  const search = await timed('search', () => call('search', { key, quick: false }), 45_000)
  const first = search?.list?.[0]
  console.log(`[search] "${key}" results=${search?.list?.length ?? 0} first=${first?.vod_name ?? '-'} id=${first?.vod_id ?? '-'}`)
  if (!first?.vod_id) throw new Error('search 没有返回 vod_id')

  const detail = await timed('detail', () => call('detail', { ids: [String(first.vod_id)] }), 45_000)
  const item = detail?.list?.[0]
  if (!item) throw new Error('detail 没有返回影片')
  console.log(`[detail] ${item?.vod_name ?? '-'} playFrom=${item?.vod_play_from ?? '-'}`)
  const flags = String(item?.vod_play_from ?? '').split('$$$').filter(Boolean)
  const lines = String(item?.vod_play_url ?? '').split('$$$')
  const firstEpisode = String(lines[0] ?? '').split('#').filter(Boolean)[0] ?? ''
  const episodeUrl = firstEpisode.includes('$') ? firstEpisode.slice(firstEpisode.lastIndexOf('$') + 1) : firstEpisode
  if (!flags[0] || !episodeUrl) throw new Error('detail 没有返回可播放线路')
  const played = await timed('player', () => call('player', {
    flag: flags[0],
    id: episodeUrl,
    vipFlags: []
  }), 45_000)
  console.log(`[player] parse=${played?.parse ?? '-'} url=${String(played?.url ?? '').slice(0, 120)}`)
  if (!played?.url) throw new Error('player 没有返回播放地址')

  console.log('[done]')
} catch (error) {
  console.error('[fail]', error.message)
  process.exitCode = 1
} finally {
  try {
    child.stdin.write(`${JSON.stringify({ id: nextId++, method: 'shutdown' })}\n`)
  } catch {
    // 忽略
  }
  setTimeout(() => child.kill(), 300)
}
