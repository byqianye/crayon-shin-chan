/**
 * Spider 真实性矩阵执行器。
 *
 * 每个用例单独起一个 Node 进程（内部再起独立 JVM），避免一个会卡死/崩溃的
 * 第三方 jar 拖垮整批测试。真实源用例由 spider-tests/cases.json 提供；
 * 合成契约用例由 spider-contract.mjs 生成后复用同一套执行器。
 *
 * 用法：
 *   node scripts/spider-matrix.mjs --cases spider-tests/cases.json --out spider-reports
 *   node scripts/spider-matrix.mjs --only csp_Mp4Mov,csp_New6v --keyword 庆余年
 */
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawn } from 'node:child_process'
import { bundleSpiderService, defaultJava, projectRoot } from './lib/spider-bundle.mjs'

const root = projectRoot()

function parseArgs(argv) {
  const args = {}
  for (let i = 0; i < argv.length; i++) {
    const item = argv[i]
    if (!item.startsWith('--')) continue
    const key = item.slice(2)
    const next = argv[i + 1]
    if (!next || next.startsWith('--')) args[key] = true
    else {
      args[key] = next
      i++
    }
  }
  return args
}

function errorText(error) {
  if (error instanceof Error) return error.message
  return String(error)
}

function classify(record) {
  const a = record.actions ?? {}
  const allErrors = Object.values(a)
    .map((item) => item?.error ?? '')
    .filter(Boolean)
    .join(' | ')

  if (/ClassNotFoundException|NoClassDefFoundError|Class not found|Could not find class/i.test(allErrors)) {
    return 'missing_class'
  }
  if (/not a Spider subclass|is not a Spider subclass/i.test(allErrors)) return 'wrong_type'
  if (/UnsatisfiedLinkError|loadLibrary|native-lib|\.so\b/i.test(allErrors)) return 'native'
  if (/NoSuchMethodError|Stub!|AbstractMethodError|NoSuchFieldError/i.test(allErrors)) return 'android_api'
  if (/SSLHandshakeException|SSLException|PKIX|certificate|TLS/i.test(allErrors)) return 'tls'
  if (/timeout|timed out|超时/i.test(allErrors)) return 'timeout'
  if (a.home?.type === 'string' || a.search?.type === 'string' || a.detail?.type === 'string') {
    return 'invalid_json'
  }
  if (!a.search || !a.search.ok) return 'search_error'
  if (!a.search.count) {
    if (a.category?.count) {
      return a.detail?.ok && a.player?.ok ? 'category_only' : 'search_empty_category_ok'
    }
    return 'empty_search'
  }
  if (!a.detail || !a.detail.ok) return 'detail_error'
  if (!a.detail.lines) return 'empty_play_fields'
  if (!a.player || a.player.skipped) return 'empty_play_fields'
  if (!a.player.ok) return 'empty_player'
  if (a.home && !a.home.ok) return 'home_error'
  return 'usable'
}

function statusCell(action) {
  if (!action) return '-'
  if (action.skipped) return '跳过'
  if (!action.ok) return `✗ ${String(action.error ?? '失败').slice(0, 80)}`
  if (typeof action.classes === 'number') return `✓ ${action.classes} 类`
  if (typeof action.count === 'number') return `✓ ${action.count} 条 / ${action.ms}ms`
  if (typeof action.lines === 'number') return `✓ ${action.lines} 线 / ${action.episodes} 集`
  if (action.url) return `✓ ${action.ms}ms`
  return `✓ ${action.ms}ms`
}

function markdown(report) {
  const lines = [
    '# Spider 真实性矩阵',
    '',
    `- 生成时间：${report.generatedAt}`,
    `- 关键词：${report.keyword}`,
    `- 用例：${report.results.length}`,
    `- 汇总：${Object.entries(report.summary)
      .map(([key, value]) => `${key}=${value}`)
      .join('，')}`,
    '',
    '| # | 类 | 分类 | home | category | search | detail | player | 备注 |',
    '| --- | --- | --- | --- | --- | --- | --- | --- | --- |'
  ]
  report.results.forEach((item, index) => {
    const note = Object.values(item.actions ?? {})
      .map((action) => action?.error)
      .filter(Boolean)
      .join('；')
    lines.push(
      `| ${index + 1} | ${item.className} | ${item.category} | ${statusCell(item.actions?.home)} | ${statusCell(item.actions?.category)} | ${statusCell(item.actions?.search)} | ${statusCell(item.actions?.detail)} | ${statusCell(item.actions?.player)} | ${String(note || item.harnessError || '').replace(/\|/g, '\\|').slice(0, 180)} |`
    )
  })
  lines.push('')
  lines.push(
    '> 分类含义：usable=全链路可用；category_only=搜索为空但分类页可取且详情/播放可用；search_empty_category_ok=搜索为空但分类页有内容；missing_class/wrong_type=缺类或类型不对；android_api=Android API 缺失；native=native 加固；timeout=超时；tls=TLS/证书；empty_play_fields=详情没有播放字段；empty_search=搜索和分类都为空；invalid_json=返回不是 JSON；home_error=仅首页失败。'
  )
  return lines.join('\n') + '\n'
}

function runNode(script, env, timeoutMs = 180_000) {
  return new Promise((resolvePromise) => {
    const child = spawn(process.execPath, [script], {
      cwd: root,
      env,
      stdio: 'inherit',
      windowsHide: true
    })
    let settled = false
    const finish = (code) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      resolvePromise(code)
    }
    const timer = setTimeout(() => {
      if (process.platform === 'win32') {
        spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' })
      } else {
        child.kill('SIGKILL')
      }
      finish(124)
    }, timeoutMs)
    child.once('exit', (code) => finish(code ?? 1))
    child.once('error', () => finish(1))
  })
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  const casesPath = resolve(args.cases || join(root, 'spider-tests', 'cases.json'))
  const outDir = resolve(args.out || join(tmpdir(), `spider-matrix-${Date.now()}`))
  const caseData = JSON.parse(await readFile(casesPath, 'utf8'))
  const keyword = String(args.keyword || caseData.keyword || '庆余年')
  const timeout = Number(args.timeout || caseData.timeout || 20)
  const only = args.only ? new Set(String(args.only).split(',').map((x) => x.trim())) : null
  const cases = (caseData.cases ?? []).filter((item) => !only || only.has(item.key))

  if (!cases.length) throw new Error(`没有可运行的用例：${casesPath}`)

  const java = defaultJava()
  if (!existsSync(java)) {
    throw new Error(`找不到 java：${java}。请设置 JAVA_HOME 或 SPIDER_JAVA`)
  }
  if (!existsSync(join(root, 'spider-host', 'out', 'host', 'SpiderHost.class'))) {
    throw new Error('缺少 spider-host/out，请先运行 pnpm spider:build')
  }

  const temp = await mkdtemp(join(tmpdir(), 'crayon-spider-matrix-'))
  const userdata = join(temp, 'userdata')
  await mkdir(userdata, { recursive: true })
  const results = []

  try {
    const bundle = await bundleSpiderService(temp)
    const runner = join(temp, 'case-runner.cjs')
    await writeFile(runner, runnerSource(), 'utf8')

    for (let index = 0; index < cases.length; index++) {
      const item = cases[index]
      const caseFile = join(temp, `case-${index}.json`)
      const resultFile = join(temp, `result-${index}.json`)
      await writeFile(
        caseFile,
        JSON.stringify({
          ...item,
          jar: item.jar || caseData.jar,
          keyword,
          timeout
        }),
        'utf8'
      )
      console.log(`\n[${index + 1}/${cases.length}] ${item.className ?? item.class}`)
      const code = await runNode(runner, {
        ...process.env,
        SPIDER_BUNDLE: bundle,
        CASE_FILE: caseFile,
        RESULT_FILE: resultFile,
        SPIDER_JAVA: java,
        SPIDER_PROJECT_ROOT: root,
        SPIDER_HOST_ROOT: join(root, 'spider-host'),
        SPIDER_DEX_TOOLS:
          process.env.SPIDER_DEX_TOOLS || join(root, 'spider-host', 'dex-tools', 'dex-tools-v2.4'),
        SPIDER_USERDATA: userdata
      })

      let record
      if (existsSync(resultFile)) {
        record = JSON.parse(await readFile(resultFile, 'utf8'))
      } else {
        record = {
          key: item.key,
          className: item.className ?? item.class,
          actions: {},
          harnessError: code === 124 ? '用例进程超时（180s）' : `用例进程退出码 ${code}`
        }
      }
      record.category = classify(record)
      results.push(record)
      console.log(`  -> ${record.category}`)
    }
  } finally {
    try {
      await rm(temp, { recursive: true, force: true })
    } catch {
      // 临时目录由系统回收
    }
  }

  const summary = {}
  for (const item of results) summary[item.category] = (summary[item.category] ?? 0) + 1
  const report = {
    generatedAt: new Date().toISOString(),
    keyword,
    casesPath,
    results,
    summary
  }
  await mkdir(outDir, { recursive: true })
  await writeFile(join(outDir, 'spider-matrix.json'), JSON.stringify(report, null, 2), 'utf8')
  await writeFile(join(outDir, 'spider-matrix.md'), markdown(report), 'utf8')
  console.log(`\n[spider-matrix] ${JSON.stringify(summary)}`)
  console.log(`[spider-matrix] 报告：${join(outDir, 'spider-matrix.md')}`)
}

function runnerSource() {
  return `const fs = require('node:fs')
const spider = require(process.env.SPIDER_BUNDLE)
const item = JSON.parse(fs.readFileSync(process.env.CASE_FILE, 'utf8'))
const site = {
  key: 'matrix-' + item.key,
  name: item.name || item.key,
  type: 3,
  api: item.api || ('csp_' + item.key),
  ext: item.ext || '',
  spiderClass: item.className,
  jar: item.jar,
  searchable: true,
  quickSearch: true,
  filterable: true,
  playerType: 1,
  sourceId: 'matrix',
  sourceName: 'matrix',
  enabled: true,
  timeout: item.timeout
}
const record = { key: item.key, name: site.name, className: site.spiderClass, actions: {} }
const timeoutMs = Number(item.timeout || 20) * 1000
const typeOf = (v) => v === null ? 'null' : Array.isArray(v) ? 'array' : typeof v
async function step(name, fn) {
  const started = Date.now()
  try {
    const value = await fn()
    record.actions[name] = { ok: true, ms: Date.now() - started }
    return value
  } catch (error) {
    record.actions[name] = { ok: false, ms: Date.now() - started, error: String(error && error.message || error) }
    return undefined
  }
}
async function main() {
  const home = await step('home', () => spider.spiderCall(site, 'home', { filter: true }, timeoutMs))
  if (home !== undefined) {
    if (!home || typeof home !== 'object') Object.assign(record.actions.home, { type: typeOf(home), raw: String(home).slice(0, 300) })
    else Object.assign(record.actions.home, { type: 'object', classes: Array.isArray(home.class) ? home.class.length : 0, list: Array.isArray(home.list) ? home.list.length : 0 })
  } else {
    Object.assign(record.actions.home, { count: 0, raw: 'undefined' })
  }
  const search = await step('search', () => spider.spiderCall(site, 'search', { key: item.keyword, quick: false }, timeoutMs))
  const list = search && typeof search === 'object' && Array.isArray(search.list) ? search.list : []
  if (search !== undefined) {
    if (!search || typeof search !== 'object') Object.assign(record.actions.search, { type: typeOf(search), count: 0, raw: String(search).slice(0, 300) })
    else Object.assign(record.actions.search, { type: 'object', count: list.length, first: list[0] ? { id: String(list[0].vod_id || list[0].id || ''), name: String(list[0].vod_name || list[0].name || '') } : null, ...(list.length ? {} : { raw: JSON.stringify(search).slice(0, 300) }) })
  } else {
    Object.assign(record.actions.search, { count: 0, raw: 'undefined' })
  }
  let picked = list[0] || null
  if (!picked && home && typeof home === 'object' && Array.isArray(home.class) && home.class.length) {
    const tid = String(home.class[0].type_id || home.class[0].id || '')
    const category = await step('category', () => spider.spiderCall(site, 'category', { tid, pg: '1', filter: true, extend: {} }, timeoutMs))
    const catList = category && typeof category === 'object' && Array.isArray(category.list) ? category.list : []
    if (category !== undefined) {
      if (!category || typeof category !== 'object') Object.assign(record.actions.category, { type: typeOf(category), count: 0, raw: String(category).slice(0, 300) })
      else Object.assign(record.actions.category, { type: 'object', count: catList.length })
    } else {
      Object.assign(record.actions.category, { count: 0, raw: 'undefined' })
    }
    picked = catList[0] || null
  }
  if (!picked) {
    record.actions.detail = { ok: false, skipped: true, error: '搜索和分类都没有结果，跳过详情' }
    record.actions.player = { ok: false, skipped: true, error: '搜索和分类都没有结果，跳过播放' }
    return
  }
  const detail = await step('detail', () => spider.spiderCall(site, 'detail', { ids: [String(picked.vod_id || picked.id || '')] }, timeoutMs))
  if (detail !== undefined) {
    if (!detail || typeof detail !== 'object') Object.assign(record.actions.detail, { type: typeOf(detail), lines: 0, episodes: 0, raw: String(detail).slice(0, 300) })
    else {
      const raw = Array.isArray(detail.list) ? detail.list[0] : null
      const flags = raw ? String(raw.vod_play_from || '').split('$$$').filter(Boolean) : []
      const groups = raw ? String(raw.vod_play_url || '').split('$$$') : []
      Object.assign(record.actions.detail, {
        type: 'object',
        name: raw ? String(raw.vod_name || '') : '',
        lines: flags.length,
        episodes: groups.reduce((n, g) => n + g.split('#').filter(Boolean).length, 0)
      })
    }
  }
  const raw = detail && typeof detail === 'object' && Array.isArray(detail.list) ? detail.list[0] : null
  const flags = raw ? String(raw.vod_play_from || '').split('$$$').filter(Boolean) : []
  const groups = raw ? String(raw.vod_play_url || '').split('$$$') : []
  let played = null
  for (let i = 0; i < Math.min(flags.length, 3); i++) {
    const episode = (groups[i] || '').split('#').filter(Boolean)[0] || ''
    const id = episode.includes('$') ? episode.slice(episode.lastIndexOf('$') + 1) : episode
    if (!id) continue
    const started = Date.now()
    try {
      played = await spider.spiderPlayerUrl(site, flags[i], id)
      record.actions.player = { ok: Boolean(played.ok), ms: Date.now() - started, url: played.url ? String(played.url).slice(0, 120) : '', parse: played.parseName || '', error: played.error || null }
      if (played.ok) break
    } catch (error) {
      record.actions.player = { ok: false, ms: Date.now() - started, error: String(error && error.message || error) }
    }
  }
  if (!record.actions.player) {
    record.actions.player = { ok: false, skipped: true, error: '详情没有线路或剧集，无法测试播放' }
  }
}
main().then(async () => {
  fs.writeFileSync(process.env.RESULT_FILE, JSON.stringify(record, null, 2))
  await spider.stopSpiderHost()
  process.exit(0)
}).catch(async (error) => {
  record.harnessError = String(error && error.message || error)
  fs.writeFileSync(process.env.RESULT_FILE, JSON.stringify(record, null, 2))
  try { await spider.stopSpiderHost() } catch {}
  process.exit(1)
})
`
}

main().catch((error) => {
  console.error(`[spider-matrix] ${errorText(error)}`)
  process.exit(1)
})
