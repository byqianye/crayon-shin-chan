import { app } from 'electron'
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { promises as fs } from 'node:fs'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import type {
  ParseResult,
  Site,
  SpiderProbeResult,
  SpiderRuntimeStatus,
  Vod
} from '@shared/types'

/**
 * TVBox Spider 宿主。
 *
 * 这里不把 Dalvik 塞进 Electron：主进程只负责按需拉起一个常驻 JVM，
 * 所有 jar 站点共用一个进程；空闲 5 分钟自动退出，控制内存占用。
 */

const IDLE_MS = 5 * 60_000
const HOST_TIMEOUT = 15_000
const JVM_ARGS = [
  '-Xms16m',
  '-Xmx192m',
  '-XX:+UseSerialGC',
  '-XX:TieredStopAtLevel=1',
  '-XX:MaxMetaspaceSize=96m',
  '-XX:ReservedCodeCacheSize=48m',
  // dex2jar 对部分大型 switch 生成的 stackmap 不完整，Java 17 下用
  // -noverify 跳过字节码校验；spider 本身就是用户导入的第三方代码。
  '-noverify',
  '-Dfile.encoding=UTF-8'
]

interface PendingCall {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  timer: NodeJS.Timeout
}

function hostRoot(): string {
  if (process.env.SPIDER_HOST_ROOT) return process.env.SPIDER_HOST_ROOT
  return app.isPackaged
    ? join(process.resourcesPath, 'spider-host')
    : join(app.getAppPath(), 'spider-host')
}

function javaExe(): string {
  if (process.env.SPIDER_JAVA) return process.env.SPIDER_JAVA
  const bundled = join(hostRoot(), 'runtime', 'bin', 'java.exe')
  if (existsSync(bundled)) return bundled
  if (process.env.JAVA_HOME) {
    const fromHome = join(process.env.JAVA_HOME, 'bin', 'java.exe')
    if (existsSync(fromHome)) return fromHome
  }
  return 'java'
}

function classPath(): string {
  const base = hostRoot()
  const lib = join(base, 'lib')
  // android.jar 提供 Android 类；json-20090211 是仍会把数字 id 转字符串的
  // 老版 org.json（新版 getString 会直接抛类型错误）。
  return `${join(base, 'out')};${join(lib, 'android-4.1.1.4.jar')};${join(lib, 'json-20090211.jar')};${join(lib, '*')}`
}

class SpiderHostClient {
  private child: ChildProcessWithoutNullStreams | null = null
  private ready: Promise<void> | null = null
  private pending = new Map<number, PendingCall>()
  private nextId = 1
  private opened = new Set<string>()
  private idleTimer: NodeJS.Timeout | null = null

  private touch(): void {
    if (this.idleTimer) clearTimeout(this.idleTimer)
    this.idleTimer = setTimeout(() => void this.stop(), IDLE_MS)
  }

  private async start(): Promise<void> {
    if (this.child) return
    const child = spawn(javaExe(), [...JVM_ARGS, '-cp', classPath(), 'host.SpiderHost'], {
      windowsHide: true,
      stdio: ['pipe', 'pipe', 'pipe']
    })
    this.child = child
    this.opened.clear()

    const ready = new Promise<void>((resolve, reject) => {
      let settled = false
      const finish = (error?: Error): void => {
        if (settled) return
        settled = true
        clearTimeout(timer)
        if (error) reject(error)
        else resolve()
      }
      const timer = setTimeout(() => {
        finish(new Error('Spider 宿主启动超时'))
        child.kill()
      }, HOST_TIMEOUT)

      createInterface({ input: child.stdout }).on('line', (line) => {
        let message: {
          event?: string
          id?: number
          ok?: boolean
          data?: unknown
          error?: string
        }
        try {
          message = JSON.parse(line)
        } catch {
          console.warn('[spider] 宿主输出无法解析：', line.slice(0, 200))
          return
        }
        if (message.event === 'ready') {
          finish()
          return
        }
        if (typeof message.id !== 'number') return
        const waiter = this.pending.get(message.id)
        if (!waiter) return
        this.pending.delete(message.id)
        clearTimeout(waiter.timer)
        if (message.ok) waiter.resolve(message.data)
        else waiter.reject(new Error(message.error || 'Spider 调用失败'))
      })

      child.once('error', (error) => {
        this.child = null
        this.ready = null
        finish(error)
      })
      child.once('exit', () => {
        if (!settled) finish(new Error('Spider 宿主提前退出'))
        this.child = null
        this.ready = null
        this.opened.clear()
        for (const waiter of this.pending.values()) {
          clearTimeout(waiter.timer)
          waiter.reject(new Error('Spider 宿主已退出'))
        }
        this.pending.clear()
      })
    })

    child.stderr.on('data', (chunk: Buffer) => {
      if (process.env.SPIDER_DEBUG) process.stderr.write(`[spider] ${chunk.toString()}`)
    })
    this.ready = ready
    try {
      await ready
      this.touch()
    } catch (error) {
      this.ready = null
      throw error
    }
  }

  private async ensureReady(): Promise<void> {
    if (!this.ready) this.ready = this.start()
    await this.ready
  }

  private send<T>(method: string, params: Record<string, unknown>, timeoutMs: number): Promise<T> {
    const child = this.child
    if (!child) throw new Error('Spider 宿主未启动')
    const id = this.nextId++
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`Spider ${method} 超时（${Math.round(timeoutMs / 1000)}s）`))
      }, timeoutMs)
      this.pending.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timer
      })
      try {
        child.stdin.write(`${JSON.stringify({ id, method, params })}\n`)
      } catch (error) {
        clearTimeout(timer)
        this.pending.delete(id)
        reject(error as Error)
      }
    })
  }

  async call<T>(
    site: Site,
    method: string,
    params: Record<string, unknown> = {},
    timeoutMs = 35_000
  ): Promise<T> {
    if (!site.spiderClass) throw new Error('该站点没有 Spider 实现类')
    await this.ensureReady()
    if (!this.opened.has(site.key)) {
      const artifacts = await prepareSpiderJar(site)
      const ext = await prepareSiteExt(site, artifacts.raw)
      await this.send('open', {
        site: site.key,
        jar: artifacts.jar,
        raw: artifacts.raw,
        class: site.spiderClass,
        ext
      }, 90_000)
      this.opened.add(site.key)
    }
    this.touch()
    return this.send<T>(method, { site: site.key, ...params }, timeoutMs)
  }

  async stop(): Promise<void> {
    if (this.idleTimer) {
      clearTimeout(this.idleTimer)
      this.idleTimer = null
    }
    const child = this.child
    this.child = null
    this.ready = null
    this.opened.clear()
    for (const waiter of this.pending.values()) {
      clearTimeout(waiter.timer)
      waiter.reject(new Error('Spider 宿主已退出'))
    }
    this.pending.clear()
    if (!child) return
    try {
      child.stdin.write(`${JSON.stringify({ id: 0, method: 'shutdown', params: {} })}\n`)
    } catch {
      // 管道已关闭
    }
    await new Promise((resolve) => setTimeout(resolve, 200))
    if (!child.killed) child.kill()
  }

  /** 只验证 JVM 与宿主协议能跑通，不加载任何站点 */
  async ping(): Promise<unknown> {
    await this.ensureReady()
    this.touch()
    return this.send('ping', {}, 10_000)
  }
}

const host = new SpiderHostClient()
interface SpiderArtifacts {
  jar: string
  raw: string
}

const preparing = new Map<string, Promise<SpiderArtifacts>>()

function findDexToolsLib(): string {
  const configured = process.env.SPIDER_DEX_TOOLS
  const candidates = configured
    ? [configured]
    : [join(hostRoot(), 'dex-tools'), join(hostRoot(), 'dex-tools', 'dex-tools-v2.4')]
  for (const candidate of candidates) {
    const lib = join(candidate, 'lib')
    if (existsSync(lib)) return lib
  }
  throw new Error(
    '缺少 dex2jar。请先运行 spider-host/fetch-tools.ps1，或设置 SPIDER_DEX_TOOLS 指向 dex-tools 目录'
  )
}

function zipStart(buffer: Buffer): number {
  for (let i = 0; i <= buffer.length - 4; i++) {
    if (buffer[i] === 0x50 && buffer[i + 1] === 0x4b && buffer[i + 2] === 0x03 && buffer[i + 3] === 0x04) {
      return i
    }
  }
  return -1
}

function looksConverted(buffer: Buffer): boolean {
  const text = buffer.toString('latin1')
  return !text.includes('classes.dex') && text.includes('.class')
}

async function runDex2Jar(input: string, output: string): Promise<void> {
  const lib = findDexToolsLib()
  await new Promise<void>((resolve, reject) => {
    const child = spawn(
      javaExe(),
      ['-cp', join(lib, '*'), 'com.googlecode.dex2jar.tools.Dex2jarCmd', '-f', input, '-o', output],
      { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] }
    )
    let stderr = ''
    child.stderr.on('data', (chunk: Buffer) => {
      stderr += chunk.toString()
    })
    const timer = setTimeout(() => {
      child.kill()
      reject(new Error('dex2jar 转换超时'))
    }, 90_000)
    child.once('error', (error) => {
      clearTimeout(timer)
      reject(error)
    })
    child.once('exit', (code) => {
      clearTimeout(timer)
      if (code === 0 && existsSync(output)) resolve()
      else {
        void fs.unlink(output).catch(() => undefined)
        reject(new Error(`dex2jar 转换失败（${code}）：${stderr.slice(-400)}`))
      }
    })
  })
}

async function prepareSpiderJar(site: Site): Promise<SpiderArtifacts> {
  const raw = (site.jar ?? '').trim()
  if (!raw) throw new Error('配置没有提供 spider 文件')
  const key = createHash('md5').update(`${site.sourceId}|${raw}`).digest('hex')
  const existing = preparing.get(key)
  if (existing) return existing

  const task = (async (): Promise<SpiderArtifacts> => {
    const dir = join(app.getPath('userData'), 'spider-cache')
    await fs.mkdir(dir, { recursive: true })
    const output = join(dir, `${key}.jar`)
    const rawCache = join(dir, `${key}.raw.zip`)
    if (existsSync(output) && existsSync(rawCache)) {
      const stat = await fs.stat(output)
      if (stat.size > 0) return { jar: output, raw: rawCache }
    }

    let input = raw
    if (!/^[a-zA-Z]:[\\/]|^\//.test(raw)) {
      const download = join(dir, `${key}.download`)
      if (!existsSync(download)) {
        const controller = new AbortController()
        const timer = setTimeout(() => controller.abort(), 60_000)
        try {
          const response = await fetch(raw.replace(/;md5;.*$/i, ''), {
            signal: controller.signal,
            headers: { 'User-Agent': 'Mozilla/5.0' }
          })
          if (!response.ok) throw new Error(`下载 spider 失败：HTTP ${response.status}`)
          await fs.writeFile(download, Buffer.from(await response.arrayBuffer()))
        } finally {
          clearTimeout(timer)
        }
      }
      input = download
    }

    const buffer = await fs.readFile(input)
    const start = zipStart(buffer)
    if (start < 0) throw new Error('spider 文件不是有效的 zip/dex')
    const normalized = buffer.subarray(start)
    await fs.writeFile(rawCache, normalized)
    if (looksConverted(normalized)) return { jar: rawCache, raw: rawCache }

    await runDex2Jar(rawCache, output)
    return { jar: output, raw: rawCache }
  })()

  preparing.set(key, task)
  try {
    return await task
  } finally {
    preparing.delete(key)
  }
}

const guardJars = new Map<string, Promise<boolean>>()

async function isFishGuardJar(path: string): Promise<boolean> {
  const existing = guardJars.get(path)
  if (existing) return existing
  const task = fs.readFile(path).then((buffer) =>
    buffer.includes(Buffer.from('FishGuard-v8.so')) ||
    buffer.includes(Buffer.from('FishGuard-v7.so'))
  ).catch(() => false)
  guardJars.set(path, task)
  return task
}

function looksEncryptedExt(ext: string): boolean {
  const text = ext.trim()
  if (!text) return false
  return !/^[{[]/.test(text) && !/^https?:\/\//i.test(text)
}

async function runFishGuardBridge(rawPath: string, ext: string, dir: string, key: string): Promise<string> {
  const inputFile = join(dir, `${key}.ext.input`)
  await fs.writeFile(inputFile, ext, 'utf8')
  try {
    const classpath = `${join(hostRoot(), 'unidbg-out')};${join(hostRoot(), 'unidbg-libs', '*')}`
    return await new Promise<string>((resolve, reject) => {
      const child = spawn(javaExe(), [
        '-Xmx256m',
        '-cp',
        classpath,
        'host.FishGuardBridge',
        '--jar',
        rawPath,
        '--method',
        'extDe',
        '--input-file',
        inputFile
      ], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
      let stdout = ''
      let stderr = ''
      const timer = setTimeout(() => {
        child.kill()
        reject(new Error('FishGuard 解密超时'))
      }, 90_000)
      child.stdout.on('data', (chunk: Buffer) => { stdout += chunk.toString('utf8') })
      child.stderr.on('data', (chunk: Buffer) => { stderr += chunk.toString('utf8') })
      child.once('error', (error) => {
        clearTimeout(timer)
        reject(error)
      })
      child.once('exit', (code) => {
        clearTimeout(timer)
        const line = stdout.split(/\r?\n/).reverse().find((item) => item.trim().startsWith('{"ok"'))
        if (!line) {
          reject(new Error(`FishGuard 解密失败（${code}）：${stderr.slice(-300)}`))
          return
        }
        try {
          const payload = JSON.parse(line) as { ok: boolean; text?: string; error?: string }
          if (!payload.ok || typeof payload.text !== 'string') {
            reject(new Error(payload.error || 'FishGuard 解密返回失败'))
            return
          }
          resolve(payload.text)
        } catch (error) {
          reject(error as Error)
        }
      })
    })
  } finally {
    await fs.unlink(inputFile).catch(() => undefined)
  }
}

async function prepareSiteExt(site: Site, rawPath: string): Promise<string> {
  const ext = (site.ext ?? '').trim()
  if (!looksEncryptedExt(ext)) return ext
  try {
    if (!(await isFishGuardJar(rawPath))) return ext
    const dir = join(app.getPath('userData'), 'spider-cache')
    const key = createHash('md5').update(`${rawPath}|${ext}`).digest('hex')
    const cache = join(dir, `${key}.ext.txt`)
    if (existsSync(cache)) return (await fs.readFile(cache, 'utf8')).trim()
    const result = await runFishGuardBridge(rawPath, ext, dir, key)
    await fs.writeFile(cache, result, 'utf8')
    return result
  } catch (error) {
    if (process.env.SPIDER_DEBUG) console.warn('[spider] ext 解密失败，继续用原 ext：', error)
    return ext
  }
}

export function spiderItemToVod(raw: Record<string, unknown>, site: Site): Vod {
  const s = (value: unknown): string => (value === undefined || value === null ? '' : String(value).trim())
  return {
    vod_id: s(raw.vod_id ?? raw.id),
    vod_name: s(raw.vod_name ?? raw.name),
    vod_pic: s(raw.vod_pic ?? raw.pic),
    vod_remarks: s(raw.vod_remarks ?? raw.remarks),
    vod_year: s(raw.vod_year ?? raw.year),
    vod_area: s(raw.vod_area ?? raw.area),
    vod_actor: s(raw.vod_actor ?? raw.actor),
    vod_director: s(raw.vod_director ?? raw.director),
    type_name: s(raw.type_name ?? raw.class),
    vod_score: s(raw.vod_score ?? raw.score),
    siteKey: site.key,
    siteName: site.name,
    sourceId: site.sourceId
  }
}

export async function spiderSearch(site: Site, wd: string, limit: number): Promise<Vod[]> {
  const payload = await host.call<{ list?: Array<Record<string, unknown>> }>(
    site,
    'search',
    { key: wd, quick: false },
    (site.timeout ?? 35) * 1000
  )
  const list = Array.isArray(payload?.list) ? payload.list : []
  return list
    .slice(0, limit)
    .map((item) => spiderItemToVod(item, site))
    .filter((item) => item.vod_id && item.vod_name)
}

export async function spiderCall<T = unknown>(
  site: Site,
  method: string,
  params: Record<string, unknown> = {},
  timeoutMs?: number
): Promise<T> {
  return host.call<T>(site, method, params, timeoutMs ?? (site.timeout ?? 35) * 1000)
}

/** 用户打开某个 Spider 站点时预热：提前完成 jar 转换/解密，首次搜索少等几秒 */
export async function prewarmSpider(site: Site): Promise<void> {
  if (!site.spiderClass) return
  await host.call(site, 'home', { filter: true }, 120_000)
}

export function makeSpiderUrl(siteKey: string, flag: string, id: string): string {
  return `spider://${encodeURIComponent(siteKey)}/${encodeURIComponent(flag)}/${encodeURIComponent(id)}`
}

export function isSpiderUrl(raw: string): boolean {
  return /^spider:\/\//i.test((raw ?? '').trim())
}

export function parseSpiderUrl(raw: string): { siteKey: string; flag: string; id: string } | null {
  const match = /^spider:\/\/([^/]+)\/([^/]+)\/([\s\S]+)$/i.exec((raw ?? '').trim())
  if (!match) return null
  try {
    return {
      siteKey: decodeURIComponent(match[1]),
      flag: decodeURIComponent(match[2]),
      id: decodeURIComponent(match[3])
    }
  } catch {
    return null
  }
}

export async function spiderPlayerUrl(site: Site, flag: string, id: string): Promise<ParseResult> {
  const started = Date.now()
  const payload = await spiderCall<{ parse?: number; url?: string; header?: Record<string, string> }>(
    site,
    'player',
    { flag, id, vipFlags: [] },
    60_000
  )
  const url = typeof payload?.url === 'string' ? payload.url.trim() : ''
  if (!url) {
    return {
      ok: false,
      url: '',
      direct: false,
      ms: Date.now() - started,
      attempts: [],
      error: 'Spider 没有返回播放地址'
    }
  }
  if (Number(payload?.parse) === 1) {
    return {
      ok: false,
      url,
      direct: false,
      parseName: 'Spider',
      ms: Date.now() - started,
      attempts: [],
      error: 'Spider 返回的是解析页，需要继续走解析接口'
    }
  }
  return {
    ok: true,
    url,
    direct: false,
    parseName: 'Spider',
    ms: Date.now() - started,
    attempts: []
  }
}

export async function stopSpiderHost(): Promise<void> {
  await host.stop()
}

/**
 * 打包后最容易出问题的是 extraResources 没带上 runtime/lib/dex-tools。
 * 这里把结果显式暴露给界面，避免用户只看到「全部不支持」却不知道缺什么。
 */
export function getSpiderRuntimeStatus(): SpiderRuntimeStatus {
  const root = hostRoot()
  const bundledJava = join(root, 'runtime', 'bin', 'java.exe')
  const homeJava = process.env.JAVA_HOME ? join(process.env.JAVA_HOME, 'bin', 'java.exe') : ''
  const runtime = existsSync(bundledJava)
  const javaSource: SpiderRuntimeStatus['javaSource'] = runtime
    ? 'bundled'
    : homeJava && existsSync(homeJava)
      ? 'java_home'
      : 'system'
  const javaPath = javaExe()
  const hostClasses = existsSync(join(root, 'out', 'host', 'SpiderHost.class'))
  const libraries =
    existsSync(join(root, 'lib', 'android-4.1.1.4.jar')) &&
    existsSync(join(root, 'lib', 'json-20090211.jar'))
  let dexTools = false
  try {
    dexTools = Boolean(findDexToolsLib())
  } catch {
    dexTools = false
  }

  const missing: string[] = []
  if (!runtime) missing.push('内置 JRE')
  if (!hostClasses) missing.push('宿主 class')
  if (!libraries) missing.push('Android/JSON 依赖')
  if (!dexTools) missing.push('dex2jar')
  const available = missing.length === 0
  return {
    available,
    hostRoot: root,
    javaPath,
    javaSource,
    runtime,
    hostClasses,
    libraries,
    dexTools,
    reason: available ? undefined : `缺少 ${missing.join('、')}`
  }
}

export async function probeSpiderHost(): Promise<SpiderProbeResult> {
  const started = Date.now()
  try {
    const data = await host.ping()
    const info = data as { usedMb?: number; maxMb?: number; sites?: number }
    const detail =
      typeof info?.usedMb === 'number'
        ? `，JVM ${info.usedMb}/${info.maxMb ?? '?'}MB，已加载 ${info.sites ?? 0} 个站点`
        : ''
    return { ok: true, ms: Date.now() - started, message: `Spider 宿主正常${detail}` }
  } catch (error) {
    return { ok: false, ms: Date.now() - started, message: (error as Error).message }
  }
}
