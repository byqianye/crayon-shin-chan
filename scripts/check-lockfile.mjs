/**
 * Guard against package.json and pnpm-lock.yaml drifting apart, which makes
 * `pnpm install --frozen-lockfile` fail in CI rather than at review time.
 */
import { promises as fs } from 'node:fs'
import { join, resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const packageJson = JSON.parse(await fs.readFile(join(root, 'package.json'), 'utf8'))
const lockfile = await fs.readFile(join(root, 'pnpm-lock.yaml'), 'utf8')

const importerStart = lockfile.indexOf('\n  .:\n')
if (importerStart < 0) throw new Error('[check-lockfile] pnpm-lock.yaml 里没有根 importers 段落')

const rest = lockfile.slice(importerStart + 1)
const nextImporter = rest.slice(1).search(/\n {2}[^ \n][^\n]*:\n/)
const rootBlock = nextImporter >= 0 ? rest.slice(0, nextImporter + 1) : rest

const declared = { ...packageJson.dependencies, ...packageJson.devDependencies }
const missing = Object.keys(declared).filter((name) => {
  const key = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return !new RegExp(`^ {6}(?:'${key}'|${key}):`, 'm').test(rootBlock)
})

if (missing.length) {
  throw new Error(
    `[check-lockfile] pnpm-lock.yaml 缺少这些依赖，请运行 pnpm install：${missing.join(', ')}`
  )
}

if (/\bpackage-lock\.json\b/.test(lockfile)) {
  throw new Error('[check-lockfile] pnpm-lock.yaml 不应引用 package-lock.json')
}

console.log(`[check-lockfile] pnpm-lock.yaml 覆盖 ${Object.keys(declared).length} 个直接依赖`)
