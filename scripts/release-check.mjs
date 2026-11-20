/**
 * Release guard: keep package.json, CHANGELOG.md, electron-builder output and
 * latest.yml on the same version, then write SHA256SUMS.txt for user verification.
 */
import { createHash } from 'node:crypto'
import { promises as fs } from 'node:fs'
import { basename, join, resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const args = process.argv.slice(2)
const lite = args.includes('--lite')
const releaseDir = join(root, lite ? 'release-lite' : 'release')

function option(name, fallback = '') {
  const index = args.indexOf(name)
  return index >= 0 && args[index + 1] && !args[index + 1].startsWith('--')
    ? args[index + 1]
    : fallback
}

function fail(message) {
  throw new Error(`[release-check] ${message}`)
}

const packageJson = JSON.parse(await fs.readFile(join(root, 'package.json'), 'utf8'))
const requestedVersion = option('--version', packageJson.version).replace(/^v/, '')
if (requestedVersion !== packageJson.version) {
  fail(`版本不一致：tag/参数 ${requestedVersion}，package.json ${packageJson.version}`)
}

const changelog = await fs.readFile(join(root, 'CHANGELOG.md'), 'utf8')
const versionHeading = new RegExp(
  `^##\\s+(?:\\[)?v?${requestedVersion.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?:\\])?(?:\\s|$)`,
  'm'
)
if (!versionHeading.test(changelog)) {
  fail(`CHANGELOG.md 缺少 v${requestedVersion} 的版本段落`)
}

if (!args.includes('--source-only')) {
  const suffix = lite ? '-lite' : ''
  const required = [
    { name: `CrayonShinChan-${requestedVersion}-x64${suffix}.exe`, minSize: 10 * 1024 * 1024 },
    { name: `CrayonShinChan-${requestedVersion}-x64${suffix}.exe.blockmap`, minSize: 1024 },
    { name: `CrayonShinChan-${requestedVersion}-x64${suffix}.zip`, minSize: 10 * 1024 * 1024 },
    { name: 'latest.yml', minSize: 1 }
  ]

  for (const { name, minSize } of required) {
    const path = join(releaseDir, name)
    const stat = await fs.stat(path).catch(() => null)
    if (!stat?.isFile() || stat.size < minSize) fail(`缺少有效产物：release/${name}`)
  }

  const latest = await fs.readFile(join(releaseDir, 'latest.yml'), 'utf8')
  const latestVersion = latest.match(/^version:\s*["']?([^"'\r\n]+)["']?\s*$/m)?.[1]?.trim()
  if (latestVersion !== requestedVersion) {
    fail(`latest.yml 版本不一致：期望 ${requestedVersion}，实际 ${latestVersion ?? '缺失'}`)
  }

  if (args.includes('--write-checksums')) {
    const checksumFiles = required
      // Release 只附完整版 latest.yml，精简版清单只列实际上传的三个包文件。
      .filter(({ name }) => !lite || name !== 'latest.yml')
      .map(({ name }) => join(releaseDir, name))
    const lines = []
    for (const path of checksumFiles) {
      const hash = createHash('sha256').update(await fs.readFile(path)).digest('hex')
      lines.push(`${hash}  ${basename(path)}`)
    }
    const sums = lite ? 'SHA256SUMS-lite.txt' : 'SHA256SUMS.txt'
    await fs.writeFile(join(releaseDir, sums), `${lines.join('\n')}\n`, 'utf8')
    console.log(`[release-check] ${sums} 已生成`)
  }
}

console.log(`[release-check] v${requestedVersion}${lite ? ' lite' : ''} 版本、产物与元数据一致`)
