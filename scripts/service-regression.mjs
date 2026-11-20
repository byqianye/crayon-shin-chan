/** Offline compatibility and latency regressions with isolated data and HTTP fixtures. */
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createServer } from 'node:http'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRequire } from 'node:module'
import { esbuildExe, projectRoot } from './lib/spider-bundle.mjs'

const root = projectRoot()
const temp = await mkdtemp(join(tmpdir(), 'crayon-service-regression-'))
const requests = []
let slowClosed = false
const video = { vod_id: '1', vod_name: '适配测试片', vod_pic: '/poster.jpg', vod_content: '简介',
  vod_play_from: 'm3u8', vod_play_url: '第1集$https://example.com/1.m3u8' }
const xml = '<?xml version="1.0"?><rss><list page="1" pagecount="3"><video><id>1</id><name><![CDATA[适配测试片]]></name><pic>/poster.jpg</pic><type>动作片</type><des>简介</des><dl><dd flag="m3u8"><![CDATA[第1集$https://example.com/1.m3u8]]></dd></dl></video></list><class><ty id="6">动作片</ty></class></rss>'
const server = createServer((req, res) => {
  const url = new URL(req.url, 'http://fixture')
  requests.push(url)
  let timer
  res.once('close', () => { clearTimeout(timer); if (url.searchParams.get('wd') === 'slow') slowClosed = true })
  timer = setTimeout(() => {
    if (url.pathname === '/repo/child.json') {
      res.end(JSON.stringify({ sites: [{ key: 'child', name: 'child', type: 1, api: '../json' }],
        headers: [{ host: 'fixture.test', header: { 'X-Fixture': '1' } }], flags: ['line'],
        home: { sections: [{ title: 'child', site: 'child' }] } }))
      return
    }
    if (url.pathname === '/xml') { res.setHeader('content-type', 'application/xml'); res.end(xml); return }
    res.setHeader('content-type', 'application/json')
    const t = url.searchParams.get('t')
    res.end(JSON.stringify({ code: 1, pagecount: t === '6' ? 2 : 4,
      class: [{ type_id: '1', type_name: '电影' }, { type_id: '6', type_pid: '1', type_name: '动作片' },
        { type_id: '7', type_pid: '1', type_name: '动作电影' }],
      list: [{ ...video, vod_id: t || '1', vod_name: t ? '分类' + t : video.vod_name }] }))
  }, url.searchParams.get('wd') === 'slow' ? 2000 : 60)
})
let svc
let failed = 0
async function check(name, run) {
  try { await run(); console.log('[PASS] ' + name) }
  catch (error) { failed++; console.error('[FAIL] ' + name + ': ' + error.message) }
}
try {
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const base = 'http://127.0.0.1:' + server.address().port
  const stub = join(temp, 'electron.cjs')
  await writeFile(stub, 'module.exports={app:{isPackaged:false,getAppPath:()=>' + JSON.stringify(root) + ',getPath:()=>' + JSON.stringify(temp) + '}}')
  const entry = join(temp, 'entry.ts')
  const modules = ['cms', 'cmsApi', 'config', 'http', 'search', 'category', 'home', 'detail', 'store', 'sources', 'sites', 'spider', 'audit', 'mediaHeaders', 'multirepo', 'presets']
  await writeFile(entry, modules.map((name) => 'export * as ' + name + ' from ' + JSON.stringify(join(root, 'src/main/services', name + '.ts'))).join('\n') +
    '\nexport * as merge from ' + JSON.stringify(join(root, 'src/core/dedupe/merge.ts')) +
    '\nexport * as normalize from ' + JSON.stringify(join(root, 'src/core/dedupe/normalize.ts')) +
    '\nexport * as detect from ' + JSON.stringify(join(root, 'src/core/source/detect.ts')) +
    '\nexport * as renderer from ' + JSON.stringify(join(root, 'src/renderer/src/store/useApp.ts')))
  const bundle = join(temp, 'service.cjs')
  const build = spawnSync(esbuildExe(), [entry, '--bundle', '--platform=node', '--format=cjs',
    '--outfile=' + bundle, '--alias:electron=' + stub, '--alias:@=' + join(root, 'src/renderer/src'),
    '--tsconfig=' + join(root, 'tsconfig.node.json'), '--log-level=error'], { stdio: 'inherit' })
  assert.equal(build.status, 0, 'service bundle compiles')
  svc = createRequire(import.meta.url)(bundle)
  const ctx = { sourceId: 'fixture', sourceName: 'fixture' }
  const site = svc.config.normalizeSite({ key: 'json', name: 'JSON', type: 3, api: base + '/json' }, ctx)
  const xmlSite = svc.config.normalizeSite({ key: 'xml', name: 'XML', type: 3, api: base + '/xml' }, ctx)
  svc.store.writeStore('sources', [{ id: 'fixture', name: 'fixture', enabled: true, raw: { sites: [site, xmlSite] } }])
  svc.store.writeStore('settings', { concurrency: 2, timeout: 3 })
  await svc.sources.hydrate()
  await check('Preset counters match bundled source definitions', async () => {
    for (const preset of svc.presets.SOURCE_PRESETS) {
      assert.equal(preset.siteCount, preset.config.sites.length)
      assert.equal(preset.liveCount, preset.config.lives.length)
    }
  })
  await check('Renderer details coalesce, expire, stay bounded and reject stale cache writes', async () => {
    const store = svc.renderer.useApp
    let calls = 0
    let unblock
    globalThis.window = { api: { sites: { list: async () => [] }, detail: { get: async (_site, id) => {
      calls++
      if (id === 'deferred') await new Promise((resolve) => { unblock = resolve })
      return { ...video, vod_id: id, siteKey: 'json', lines: [] }
    } } } }
    const load = (id) => store.getState().loadDetail('json', id)
    await Promise.all([load('same'), load('same')])
    assert.equal(calls, 1)
    const now = Date.now
    try { Date.now = () => now() + 121000; await load('same') }
    finally { Date.now = now }
    assert.equal(calls, 2)
    for (let i = 0; i < 105; i++) await load(String(i))
    assert.equal(Object.keys(store.getState().details).length, 100)
    const stale = load('deferred')
    await store.getState().refreshSites()
    unblock()
    await stale
    assert.deepEqual(store.getState().details, {})
    delete globalThis.window
  })
  await check('CMS parameters replace fixed values and fill missing template slots', () => {
    const url = new URL(svc.cms.buildCmsUrl({ ...site, api: base + '/json?ac=list&pg={PG}#keep' }, { ac: 'detail', pg: 2, wd: '测试' }))
    assert.deepEqual(url.searchParams.getAll('ac'), ['detail'])
    assert.equal(url.searchParams.get('pg'), '2')
    assert.equal(url.searchParams.get('wd'), '测试')
    assert.equal(url.hash, '#keep')
  })
  await check('TVBox HTTP type 0/1 normalize to CMS while HTML rules remain HTML', () => {
    for (const type of [0, 1, '0', '1', 'xml']) {
      const normalized = svc.config.normalizeSite({ name: 'test', type, api: base + '/xml' }, ctx)
      assert.equal(normalized.unsupported, undefined)
      assert.equal(normalized.type, 3)
    }
    assert.equal(svc.config.normalizeSite({ name: 'html', type: 1, api: base, rules: { list: 'a' } }, ctx).type, 1)
    assert(svc.config.normalizeSite({ name: 'spider', type: 'spider', api: base }, ctx).unsupported)
  })
  await check('Subscription-relative API, jar, ext, live and parser URLs resolve', () => {
    const parsed = svc.config.parseConfig({ spider: './jar/main.jar', sites: [
      { key: 'relative', name: 'relative', type: 1, api: '../api.php?ac={ac}&wd={wd}' },
      { key: 'jar', name: 'jar', type: 3, api: 'csp_Test', ext: './ext/site.json' }
    ], lives: [{ name: 'live', url: './live.m3u' }], parses: [{ name: 'jx', url: './jx?url={url}' }] },
    { ...ctx, baseUrl: base + '/config/list.json' })
    assert.equal(parsed.sites[0].api, base + '/api.php?ac={ac}&wd={wd}')
    assert.equal(parsed.sites[1].jar, base + '/config/jar/main.jar')
    assert.equal(parsed.sites[1].ext, base + '/config/ext/site.json')
    assert.equal(parsed.lives[0].url, base + '/config/live.m3u')
    assert.equal(parsed.parses[0].url, base + '/config/jx?url={url}')
    assert.equal(svc.config.normalizeSite({ name: 'bare', type: 1, api: 'cms.fixture.test/api.php/provide/vod' },
      { ...ctx, baseUrl: base + '/config/list.json' }).api, 'https://cms.fixture.test/api.php/provide/vod')
  })
  await check('Multi-repo imports preserve child-relative URLs and request rules', async () => {
    const multi = await svc.multirepo.loadMultiRepo({ id: 'multi', name: 'multi' }, [{ name: 'child', url: base + '/repo/child.json' }], 3)
    assert.equal(multi.repos[0].ok, true)
    assert.equal(multi.parsed.sites[0].api, base + '/json')
    assert.equal(multi.parsed.headers[0].header['X-Fixture'], '1')
    assert.deepEqual(multi.parsed.flags, ['line'])
    assert.equal(multi.parsed.home.sections[0].site, 'child')
  })
  await check('XML search, flat classes, detail and playable episodes', async () => {
    const result = await svc.search.searchSite(xmlSite, '测试')
    assert.equal(result.ok, true, result.message)
    assert.equal(result.list[0].vod_pic, base + '/poster.jpg')
    const detail = await svc.detail.getVodDetail('xml', '1')
    assert.equal(detail.lines[0].episodes[0].url, 'https://example.com/1.m3u8')
    assert((await svc.category.listCategories()).some((c) => c.subs.some((s) => s.sites.some((v) => v.siteKey === 'xml'))))
    assert.equal((await svc.sites.testSite('xml')).ok, true)
    assert.equal((await svc.audit.auditSite('xml')).ok, true)
    assert.equal(svc.detect.detectSourceContent(xml).kind, 'cms')
  })
  await check('Search listing primes detail cache', async () => {
    await svc.search.searchSite(site, '测试')
    const before = requests.length
    const details = await Promise.all(Array.from({ length: 8 }, () => svc.detail.getVodDetail('json', '1')))
    assert.equal(details[0].vod_content, '简介')
    assert.equal(requests.length - before, 0, 'listing already returned the full detail')
  })
  await check('Cold detail requests coalesce and invalidation fetches fresh data', async () => {
    svc.cmsApi.clearCmsCache()
    let before = requests.length
    await Promise.all(Array.from({ length: 8 }, () => svc.detail.getVodDetail('json', '1')))
    assert.equal(requests.length - before, 1)
    svc.cmsApi.clearCmsCache()
    before = requests.length
    await svc.detail.getVodDetail('json', '1')
    assert.equal(requests.length - before, 1)
  })
  await check('CMS rejects HTML error pages and malformed JSON shapes', () => {
    for (const body of ['<html><body>bad gateway</body></html>', '{"data":"blocked"}', '[]', 'null']) {
      assert.throws(() => svc.cms.parseCmsPayload(body))
    }
    assert.equal(svc.cms.isCmsOk(svc.cms.parseCmsPayload('{"code":403,"msg":"blocked"}')), false)
  })
  await check('Media headers reach playlist children without leaking to other origins', () => {
    svc.mediaHeaders.registerMediaHeaders('https://media.example/a.m3u8', {
      Referer: 'https://source.example/', Cookie: 'fixture', Host: 'wrong.example', Origin: 'bad\r\nvalue'
    })
    assert.deepEqual(svc.mediaHeaders.mediaHeadersFor('https://media.example/segment.ts'), {
      Referer: 'https://source.example/', Cookie: 'fixture'
    })
    assert.equal(svc.mediaHeaders.mediaHeadersFor('https://other.example/segment.ts'), undefined)
    assert.equal(svc.mediaHeaders.mediaHeadersFor('http://media.example/segment.ts'), undefined)
    assert.equal(svc.mediaHeaders.mediaHeadersFor('invalid'), undefined)
    svc.mediaHeaders.registerMediaHeaders('https://media.example/a.m3u8')
    assert.equal(svc.mediaHeaders.mediaHeadersFor('https://media.example/segment.ts'), undefined)
  })
  await check('HTTP aborts promptly and preserves AbortError', async () => {
    const controller = new AbortController()
    const start = performance.now()
    const pending = svc.http.fetchText(base + '/json?wd=slow', { signal: controller.signal, timeout: 3 })
    setTimeout(() => controller.abort(), 80)
    await assert.rejects(pending, { name: 'AbortError' })
    assert(performance.now() - start < 500)
  })
  await check('Search cancellation closes sockets without penalizing health', async () => {
    slowClosed = false
    const start = performance.now()
    const pending = svc.search.searchAll('slow', { sites: ['json'] })
    setTimeout(() => svc.search.cancelSearch(), 100)
    const result = await pending
    await new Promise((resolve) => setTimeout(resolve, 40))
    assert.equal(result.results.length, 0)
    assert(performance.now() - start < 600, 'cancel waited for upstream timeout')
    assert.equal(slowClosed, true)
    assert.equal((await svc.store.readStore('health', {})).json.consecutiveFails, 0)
  })
  await check('HTTP search responds before cold Spiders and healthy sources go first', async () => {
    const parsed = svc.sources.getParsed('fixture')
    const previous = structuredClone(await svc.store.readStore('health', {}))
    const fake = { ...site, key: 'cold', name: 'AAA', api: 'csp_Missing', spiderClass: 'fixture.Missing', jar: undefined }
    parsed.sites.push(fake)
    svc.store.writeStore('health', {
      json: { samples: [{ ok: false, ms: 500, at: Date.now() }], consecutiveFails: 1 },
      xml: { samples: [{ ok: true, ms: 25, at: Date.now() }], consecutiveFails: 0 }
    })
    const order = []
    try {
      await svc.search.searchAll('priority', { concurrency: 1 }, (result) => order.push(result.siteKey))
      assert.deepEqual(order, ['xml', 'json', 'cold'])
    } finally {
      parsed.sites.pop()
      svc.store.writeStore('health', previous)
    }
  })
  await check('Category requests each leaf separately and retains deeper pages', async () => {
    const before = requests.length
    const page = await svc.category.browseCategory('电影', '动作', 1)
    const sent = requests.slice(before).filter((u) => u.searchParams.has('t'))
    assert(sent.every((u) => u.searchParams.getAll('t').length === 1))
    assert(sent.some((u) => u.searchParams.get('t') === '6'))
    assert(sent.some((u) => u.searchParams.get('t') === '7'))
    assert.equal(page.pageCount, 4)
  })
  await check('Concurrent home loads coalesce and stale responses cannot repopulate cache', async () => {
    svc.home.clearHomeCache()
    const before = requests.length
    await Promise.all([svc.home.listHomeSections(), svc.home.listHomeSections()])
    assert.equal(requests.length - before, 2)
    svc.home.clearHomeCache()
    const pending = svc.home.listHomeSections()
    await new Promise((resolve) => setTimeout(resolve, 20))
    svc.home.clearHomeCache()
    await pending
    const afterStale = requests.length
    await svc.home.listHomeSections()
    assert.equal(requests.length - afterStale, 2, 'stale response repopulated cache')
  })
  await check('Merge keeps 1200 distinct titles', () => {
    const list = Array.from({ length: 1200 }, (_, i) => ({ ...video, vod_id: String(i), vod_name: '作品编号' + i, siteKey: String(i % 8), type_name: '电影' }))
    const start = performance.now()
    assert.equal(svc.merge.mergeResults([{ list }]).merged.length, list.length)
    console.log('[BENCH] merge 1200 unique titles: ' + Math.round(performance.now() - start) + 'ms')
    const knownYear = list.map((vod) => ({ ...vod, vod_year: '2026' }))
    const next = performance.now()
    assert.equal(svc.merge.mergeResults([{ list: knownYear }]).merged.length, list.length)
    console.log('[BENCH] merge 1200 titles with known years: ' + Math.round(performance.now() - next) + 'ms')
  })
  await check('Indexed merge preserves the original ambiguous year/type/season matching', () => {
    const n = svc.normalize
    const titles = ['射雕英雄传', '射雕英雄传（国语）', '庆余年', '庆余年 第二季', '庆余年 S02', '很长的作品标题甲乙丙丁一', '很长的作品标题甲乙丙丁二']
    const list = Array.from({ length: 180 }, (_, i) => ({ ...video, vod_id: String(i), siteKey: 'case-' + i,
      vod_name: titles[i % titles.length], vod_year: ['', '2001', '2002'][Math.floor(i / 7) % 3],
      type_name: ['', '电影', '电视剧', '动漫'][Math.floor(i / 21) % 4] }))
    const exact = new Map()
    for (const vod of list) {
      const key = n.buildDedupeKey(vod)
      if (exact.has(key)) exact.get(key).push(vod)
      else exact.set(key, [vod])
    }
    const compatible = (a, b) => {
      const x = n.typeBucket(a.type_name), y = n.typeBucket(b.type_name)
      if (x !== 'other' && y !== 'other' && x !== y) return false
      if (n.extractSeason(a.vod_name) !== n.extractSeason(b.vod_name)) return false
      const ay = n.extractYear(a.vod_year, a.vod_name), by = n.extractYear(b.vod_year, b.vod_name)
      if (ay && by && ay !== by) return false
      return !ay || !by ? n.normalizeName(a.vod_name) === n.normalizeName(b.vod_name) : n.titleSimilarity(a.vod_name, b.vod_name) >= 0.92
    }
    const groups = []
    for (const values of exact.values()) {
      const group = groups.find((g) => compatible(g[0], values[0]))
      if (group) group.push(...values)
      else groups.push([...values])
    }
    const expected = groups.map((g) => g.map((v) => v.siteKey).sort().join(',')).sort()
    const actual = svc.merge.mergeResults([{ list }]).merged.map((v) => [...v.allSiteKeys].sort().join(',')).sort()
    assert.deepEqual(actual, expected)
  })
} finally {
  if (svc) { await svc.spider.stopSpiderHost(); await svc.store.flushStore() }
  server.closeAllConnections()
  await new Promise((resolve) => server.close(resolve))
  await rm(temp, { recursive: true, force: true })
}
if (failed) process.exitCode = 1
