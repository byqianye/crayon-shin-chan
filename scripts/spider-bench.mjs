/**
 * 对已启动的 dev:debug 窗口跑 Spider 搜索热身/计时。
 * 用法：node scripts/spider-bench.mjs [站点key] [关键词]
 */
const siteKey = process.argv[2] || 'spider_e2e'
const keyword = process.argv[3] || '庆余年'

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
      rejectEval(new Error('CDP 超时'))
    }, timeoutMs)
    pending.set(id, (message) => {
      clearTimeout(timer)
      if (message.result?.exceptionDetails) {
        rejectEval(new Error(message.result.exceptionDetails.text || '页面执行异常'))
        return
      }
      resolveEval(message.result?.result?.value)
    })
    ws.send(JSON.stringify({
      id,
      method: 'Runtime.evaluate',
      params: { expression, awaitPromise: true, returnByValue: true }
    }))
  })
}

try {
  for (let i = 0; i < 3; i++) {
    const result = await evaluate(`(async () => {
      const started = performance.now()
      const response = await window.api.search.run(${JSON.stringify(keyword)}, {
        sites: [${JSON.stringify(siteKey)}],
        limitPerSite: 10
      })
      return {
        ms: Math.round(performance.now() - started),
        ok: response.results?.[0]?.ok,
        count: response.results?.[0]?.list?.length ?? 0,
        message: response.results?.[0]?.message ?? ''
      }
    })()`)
    console.log(`[bench ${i + 1}]`, JSON.stringify(result))
  }
} finally {
  ws.close()
}
