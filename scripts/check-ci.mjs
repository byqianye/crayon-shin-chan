/**
 * Static CI configuration guard. It catches the release-critical steps being
 * removed or downgraded while keeping YAML parsing out of the dependency tree.
 */
import { promises as fs } from 'node:fs'
import { join, resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const [ciRaw, releaseRaw, packageJson, selftestRunner] = await Promise.all([
  fs.readFile(join(root, '.github', 'workflows', 'ci.yml'), 'utf8'),
  fs.readFile(join(root, '.github', 'workflows', 'release.yml'), 'utf8'),
  fs.readFile(join(root, 'package.json'), 'utf8').then(JSON.parse),
  fs.readFile(join(root, 'scripts', 'ci-selftest.mjs'), 'utf8')
])

// 工作流在 Windows 上检出后可能是 CRLF，统一成 LF 再做结构断言
const ci = ciRaw.replace(/\r\n/g, '\n')
const release = releaseRaw.replace(/\r\n/g, '\n')

const failed = []
function check(name, condition) {
  if (!condition) failed.push(name)
}

check('CI 在 push/PR 触发', /^ {2}pull_request:/m.test(ci) && /^ {2}push:/m.test(ci))
check('CI 使用 Windows runner', /runs-on: windows-latest/.test(ci))
check('CI 使用冻结锁文件', /pnpm install --frozen-lockfile/.test(ci))
check('CI 编译 Spider 宿主', /pnpm spider:build/.test(ci))
check('CI 编译 unidbg 桥', /pnpm spider:unidbg/.test(ci))
check('CI 构建生产包后运行端到端自检', /pnpm build/.test(ci) && /node scripts\/ci-selftest\.mjs/.test(ci))
check('自检直接启动 Electron 而不是 dev server', /electron\.exe/.test(selftestRunner))
check('自检等待窗口为 120 秒', /READY_TIMEOUT_MS = 120_000/.test(selftestRunner))
check('自检使用隔离数据目录', /CRAYON_DATA_DIR/.test(selftestRunner) && /CRAYON_DISABLE_BUNDLED_SOURCES/.test(selftestRunner))
check('发布工作流由 tag 或手动触发', /tags: \['v\*'\]/.test(release) && /workflow_dispatch:/.test(release))
check('发布工作流生成精简 JRE', /pnpm spider:runtime/.test(release))
check('发布前运行端到端自检', /node scripts\/ci-selftest\.mjs/.test(release))
check('发布工作流校验产物', /node scripts\/release-check\.mjs/.test(release))
check('发布工作流生成校验清单', /--write-checksums/.test(release))
check('发布工作流生成发布说明', /node scripts\/release-notes\.mjs/.test(release))
check('正式 Release 由 tag 触发', /startsWith\(github\.ref, 'refs\/tags\/v'\)/.test(release))
check('自动发布只创建 draft', /--draft/.test(release))
check('已有正式 Release 会拒绝覆盖', /已经是正式 Release，拒绝自动覆盖/.test(release))
check('发布资产缺失时报错', /没有找到可上传的发布资产/.test(release) && /发布资产不完整/.test(release))
check('packageManager 固定 pnpm 版本', packageJson.packageManager === 'pnpm@11.25.0')
check('打包脚本禁止 electron-builder 隐式发布', /--publish never/.test(packageJson.scripts['build:win']))

if (failed.length) {
  throw new Error(`[check-ci] 失败：${failed.join('、')}`)
}

console.log('[check-ci] 21 项工作流约束通过')
