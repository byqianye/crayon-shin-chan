/**
 * 把主进程的 spider 服务打成一个可在普通 Node 进程里跑的 CJS bundle。
 * electron 只用到 app.getAppPath / app.getPath，这里用桩替换掉。
 */
import { existsSync } from 'node:fs'
import { writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

const root = resolve(import.meta.dirname, '..', '..')

export function esbuildExe() {
  return (
    process.env.ESBUILD_EXE ||
    join(
      root,
      'node_modules',
      '.pnpm',
      '@esbuild+win32-x64@0.25.12',
      'node_modules',
      '@esbuild',
      'win32-x64',
      'esbuild.exe'
    )
  )
}

export async function bundleSpiderService(outDir) {
  const esbuild = esbuildExe()
  if (!existsSync(esbuild)) throw new Error(`找不到 esbuild：${esbuild}`)

  const stub = join(outDir, 'electron-stub.cjs')
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

  const bundle = join(outDir, 'spider.cjs')
  const result = spawnSync(
    esbuild,
    [
      join(root, 'src', 'main', 'services', 'spider.ts'),
      '--bundle',
      '--platform=node',
      '--format=cjs',
      `--outfile=${bundle}`,
      `--alias:electron=${stub}`,
      `--alias:@shared=${join(root, 'src', 'shared')}`,
      `--tsconfig=${join(root, 'tsconfig.node.json')}`,
      '--log-level=error'
    ],
    { stdio: 'inherit' }
  )
  if (result.status !== 0) throw new Error('esbuild 打包 spider 服务失败')
  return bundle
}

export function defaultJava() {
  if (process.env.SPIDER_JAVA) return process.env.SPIDER_JAVA
  if (process.env.JAVA_HOME) return join(process.env.JAVA_HOME, 'bin', 'java.exe')
  return join(root, 'spider-host', 'runtime', 'bin', 'java.exe')
}

export function projectRoot() {
  return root
}
