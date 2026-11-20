/**
 * CI 端到端自检入口：用生产构建直接启动 Electron，再跑 scripts/selftest.mjs。
 *
 * 为什么不复用 `electron-vite dev`：Windows runner 上 dev server 的首次冷启动
 * 实测要十几分钟（日志里 600 秒都没打印出 DevTools listening），会把自检窗口
 * 拖到超时。生产构建几秒钟就能起来，跑的还是同一份 renderer 代码。
 */
import { spawn, spawnSync } from 'node:child_process'
import { closeSync, openSync, readFileSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const electron = join(root, 'node_modules', 'electron', 'dist', 'electron.exe')
const logDir = process.env.RUNNER_TEMP || tmpdir()
const outLog = join(logDir, 'dev.log')
const errLog = join(logDir, 'dev.err.log')
const CDP = 'http://127.0.0.1:9222'
const READY_TIMEOUT_MS = 120_000

if (!existsSync(electron)) {
  console.error(`[ci-selftest] 找不到 Electron：${electron}`)
  process.exit(1)
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

try {
  const existing = await fetch(`${CDP}/json/version`, { signal: AbortSignal.timeout(1000) })
  if (existing.ok) {
    console.error('[ci-selftest] 端口 9222 已被占用，请先关闭正在运行的调试窗口')
    process.exit(1)
  }
} catch {
  // 端口空闲，继续启动
}

function tail(path, lines = 40) {
  if (!existsSync(path)) return ''
  return readFileSync(path, 'utf8').trim().split(/\r?\n/).slice(-lines).join('\n')
}

function killTree(child) {
  if (!child || child.exitCode !== null) return
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' })
  } else {
    try {
      child.kill('SIGTERM')
    } catch {
      // 进程已经退出
    }
  }
}

async function waitForCdp(child) {
  const deadline = Date.now() + READY_TIMEOUT_MS
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`Electron 在调试端口就绪前退出（code ${child.exitCode}）`)
    }
    try {
      const res = await fetch(`${CDP}/json/version`, { signal: AbortSignal.timeout(2000) })
      if (res.ok) return
    } catch {
      // 端口还没起来，继续等
    }
    await sleep(1000)
  }
  throw new Error(`调试端口 9222 在 ${READY_TIMEOUT_MS / 1000} 秒内没有就绪`)
}

const outFd = openSync(outLog, 'w')
const errFd = openSync(errLog, 'w')
const child = spawn(
  electron,
  [
    '.',
    '--remote-debugging-port=9222',
    '--no-sandbox',
    '--disable-gpu',
    '--disable-software-rasterizer',
    '--disable-dev-shm-usage'
  ],
  {
    cwd: root,
    env: { ...process.env, ELECTRON_ENABLE_LOGGING: '1' },
    stdio: ['ignore', outFd, errFd],
    windowsHide: true
  }
)

let exitCode = 1
try {
  await waitForCdp(child)
  console.log('[ci-selftest] Electron 已就绪，开始自检')
  const result = await new Promise((resolve) => {
    const selftest = spawn(process.execPath, [join(root, 'scripts', 'selftest.mjs')], {
      cwd: root,
      stdio: 'inherit',
      windowsHide: true
    })
    selftest.on('exit', (code) => resolve(code ?? 1))
  })
  exitCode = result
} catch (err) {
  console.error(`[ci-selftest] ${err instanceof Error ? err.message : String(err)}`)
  exitCode = 1
} finally {
  killTree(child)

  if (exitCode !== 0) {
    console.error('\n--- dev.log ---\n' + tail(outLog))
    console.error('\n--- dev.err.log ---\n' + tail(errLog))
  }

  closeSync(outFd)
  closeSync(errFd)
}

process.exit(exitCode)
