/**
 * Static CI configuration guard. It catches the release-critical steps being
 * removed or downgraded while keeping YAML parsing out of the dependency tree.
 */
import { promises as fs } from 'node:fs'
import { join, resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const [ciRaw, releaseRaw, liveRaw, packageJson, selftestRunner, rustPackager] = await Promise.all([
  fs.readFile(join(root, '.github', 'workflows', 'ci.yml'), 'utf8'),
  fs.readFile(join(root, '.github', 'workflows', 'release.yml'), 'utf8'),
  fs.readFile(join(root, '.github', 'workflows', 'spider-live.yml'), 'utf8'),
  fs.readFile(join(root, 'package.json'), 'utf8').then(JSON.parse),
  fs.readFile(join(root, 'scripts', 'ci-selftest.mjs'), 'utf8'),
  fs.readFile(join(root, 'native-rust', 'scripts', 'package.ps1'), 'utf8')
])

// 工作流在 Windows 上检出后可能是 CRLF，统一成 LF 再做结构断言
const ci = ciRaw.replace(/\r\n/g, '\n')
const release = releaseRaw.replace(/\r\n/g, '\n')
const live = liveRaw.replace(/\r\n/g, '\n')

const failed = []
let total = 0
function check(name, condition) {
  total++
  if (!condition) failed.push(name)
}

check('CI 在 push/PR 触发', /^ {2}pull_request:/m.test(ci) && /^ {2}push:/m.test(ci))
check('CI 使用 Windows runner', /runs-on: windows-latest/.test(ci))
check('CI 使用冻结锁文件', /pnpm install --frozen-lockfile/.test(ci))
check('CI 校验应用图标同步', /pnpm icon:check/.test(ci))
check('CI 编译 Spider 宿主', /pnpm spider:build/.test(ci))
check('CI 编译 unidbg 桥', /pnpm spider:unidbg/.test(ci))
check('CI 运行 Spider 契约矩阵', /node scripts\/spider-contract\.mjs/.test(ci))
check('CI 构建生产包后运行端到端自检', /pnpm build/.test(ci) && /node scripts\/ci-selftest\.mjs/.test(ci))
check('自检直接启动 Electron 而不是 dev server', /electron\.exe/.test(selftestRunner))
check('自检等待窗口为 120 秒', /READY_TIMEOUT_MS = 120_000/.test(selftestRunner))
check('自检使用隔离数据目录', /CRAYON_DATA_DIR/.test(selftestRunner) && /CRAYON_DISABLE_BUNDLED_SOURCES/.test(selftestRunner))
check('发布工作流由 tag 或手动触发', /tags: \['v\*'\]/.test(release) && /workflow_dispatch:/.test(release))
check('发布工作流生成精简 JRE', /pnpm spider:runtime/.test(release))
check('发布前校验应用图标', /pnpm icon:check/.test(release))
check('发布前运行离线服务回归', /node scripts\/service-regression\.mjs/.test(release))
check('发布前运行 Spider 契约', /node scripts\/spider-contract\.mjs/.test(release))
check('发布前校验锁文件与工作流', /node scripts\/check-lockfile\.mjs/.test(release) && /node scripts\/check-ci\.mjs/.test(release))
check('发布前运行端到端自检', /node scripts\/ci-selftest\.mjs/.test(release))
check('发布工作流构建 Rust 包', /pnpm package:rust/.test(release) && !/pnpm build:win:lite/.test(release))
check('发布前完成 Rust 检查与两种配置测试', /cargo check --locked/.test(release) && /cargo clippy --locked/.test(release) && /--tests/.test(release) && /--no-default-features/.test(release))
check('发布工作流校验产物', /node scripts\/release-check\.mjs/.test(release))
check('发布工作流生成校验清单', /--write-checksums/.test(release))
check('发布工作流生成发布说明', /node scripts\/release-notes\.mjs/.test(release))
check('正式 Release 由 tag 触发', /startsWith\(github\.ref, 'refs\/tags\/v'\)/.test(release))
check('自动发布只创建 draft', /--draft/.test(release))
check('已有正式 Release 会拒绝覆盖', /已经是正式 Release，拒绝自动覆盖/.test(release))
check('发布资产缺失时报错', /没有找到可上传的发布资产/.test(release) && /发布资产不完整/.test(release))
check('draft 任务在 gh 前检出仓库', /Publish draft GitHub Release[\s\S]*actions\/checkout@v4/.test(release) && /GH_REPO: \$\{\{ github\.repository \}\}/.test(release))
check('packageManager 固定 pnpm 版本', packageJson.packageManager === 'pnpm@11.25.0')
check('旧版打包脚本禁止 electron-builder 隐式发布', /--publish never/.test(packageJson.scripts['build:win:electron']))
check('默认打包入口是 Rust', /native-rust\/scripts\/package\.ps1/.test(packageJson.scripts['build:win']))
check('Rust NSIS 明确使用 UTF8 并拒绝编译警告', /\/INPUTCHARSET UTF8 \/WX/.test(rustPackager))
check('Rust 打包独立验证 ZIP 内容', /verify-package\.ps1/.test(rustPackager))
check('提供精简版打包脚本', /electron-builder\.lite\.yml/.test(packageJson.scripts['build:win:lite'] ?? ''))
check('发布资产完整性包含 Rust 安装包、ZIP 与校验清单', /rust-x64\.exe/.test(release) && /rust-x64\.zip/.test(release) && /SHA256SUMS-rust\.txt/.test(release) && /foreach \(\$file in \$files\)/.test(release))
check('CI 包含 Rust 格式、静态检查与回归', /cargo fmt/.test(ci) && /cargo clippy/.test(ci) && /cargo test/.test(ci) && /--no-default-features/.test(ci))
check('真实 Spider 矩阵由定时任务触发', /schedule:/.test(live) && /spider-matrix\.mjs/.test(live))
check('真实 Spider 矩阵报告作为 artifact 保存', /upload-artifact@v4/.test(live) && /spider-reports\//.test(live))

if (failed.length) {
  throw new Error(`[check-ci] 失败：${failed.join('、')}`)
}

console.log(`[PASS] [check-ci] ${total} 项工作流约束通过`)
