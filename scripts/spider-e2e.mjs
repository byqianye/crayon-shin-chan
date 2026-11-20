/**
 * 在已启动的 dev:debug 窗口里跑真实 Spider 全链路：
 * 添加临时配置源 → 启用一个 csp_* 站点 → search → detail → player。
 *
 * 用法：SPIDER_JAR=<spider zip/jar> node scripts/spider-e2e.mjs
 */
import { writeFile, rm } from 'node:fs/promises'
import { join, resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const jar = process.env.SPIDER_JAR || ''
if (!jar) {
  console.error('[skip] 请设置 SPIDER_JAR')
  process.exit(0)
}

const configPath = join(root, 'data', 'spider-e2e.json')
const config = {
  name: 'Spider E2E',
  spider: jar,
  sites: [
    {
      key: 'spider_e2e',
      name: 'Spider E2E 荐片',
      type: 3,
      api: 'csp_Jianpian',
      ext: 'https://api.ztcgi.com',
      searchable: 1,
      enabled: 1
    }
  ],
  lives: []
}
await writeFile(configPath, JSON.stringify(config, null, 2), 'utf8')

const tabs = await (await fetch('http://127.0.0.1:9222/json/list')).json()
const tab = tabs.find((item) => item.type === 'page')
if (!tab) throw new Error('没有找到可调试页面')

const ws = new WebSocket(tab.webSocketDebuggerUrl)
await new Promise((resolveOpen, rejectOpen) => {
  ws.addEventListener('open', resolveOpen, { once: true })
  ws.addEventListener('error', rejectOpen, { once: true })
})

let nextId = 1
const pending = new Map()
ws.addEventListener('message', (event) => {
  const message = JSON.parse(event.data)
  const waiter = pending.get(message.id)
  if (!waiter) return
  pending.delete(message.id)
  waiter(message)
})

function evaluate(expression, timeoutMs = 120_000) {
  const id = nextId++
  return new Promise((resolveEval, rejectEval) => {
    const timer = setTimeout(() => {
      pending.delete(id)
      rejectEval(new Error(`CDP 超时：${expression.slice(0, 60)}`))
    }, timeoutMs)
    pending.set(id, (message) => {
      clearTimeout(timer)
      if (message.error) {
        rejectEval(new Error(message.error.message))
        return
      }
      const result = message.result?.result
      if (message.result?.exceptionDetails) {
        rejectEval(new Error(message.result.exceptionDetails.text || '页面执行异常'))
        return
      }
      resolveEval(result?.value)
    })
    ws.send(JSON.stringify({
      id,
      method: 'Runtime.evaluate',
      params: { expression, awaitPromise: true, returnByValue: true }
    }))
  })
}

const expression = `(async () => {
  const added = await window.api.sources.add({
    name: 'Spider E2E',
    url: ${JSON.stringify(configPath)},
    kind: 'file'
  })
  await window.api.sources.sync(added.id)
  const sites = await window.api.sites.list()
  const site = sites.find((item) => item.key === 'spider_e2e')
  if (!site) throw new Error('配置同步后没有找到 spider_e2e')
  await window.api.sites.setEnabled('spider_e2e', true)

  const search = await window.api.search.run('庆余年', { sites: ['spider_e2e'] })
  const first = search.results[0]?.list?.[0]
  if (!first) throw new Error('Spider 搜索没有返回结果：' + JSON.stringify(search.results[0]))

  const detail = await window.api.detail.get('spider_e2e', first.vod_id)
  const line = detail.lines[0]
  const episode = line && line.episodes[0]
  if (!episode) throw new Error('Spider 详情没有解析出线路')

  const played = await window.api.parse.resolve(episode.url)
  return {
    site: {
      key: site.key,
      spiderClass: site.spiderClass,
      enabled: site.enabled
    },
    search: {
      total: search.total,
      ok: search.results[0]?.ok,
      count: search.results[0]?.list?.length,
      first: first.vod_name,
      id: first.vod_id
    },
    detail: {
      name: detail.vod_name,
      lines: detail.lines.length,
      episodes: line.episodes.length,
      firstUrl: episode.url
    },
    player: {
      ok: played.ok,
      url: played.url,
      parseName: played.parseName
    }
  }
})()`

try {
  const result = await evaluate(expression)
  console.log(JSON.stringify(result, null, 2))
} finally {
  ws.close()
  await rm(configPath, { force: true })
}
