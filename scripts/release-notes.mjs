/**
 * Extract the matching CHANGELOG section and wrap it in release-facing notes.
 */
import { promises as fs } from 'node:fs'
import { join, resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const args = process.argv.slice(2)
const rust = args.includes('--rust')
const index = args.indexOf('--version')
const version = index >= 0 ? args[index + 1]?.replace(/^v/, '') : ''

if (!version) {
  throw new Error('用法：node scripts/release-notes.mjs --rust --version 0.1.8 [--write]')
}

const changelog = await fs.readFile(join(root, 'CHANGELOG.md'), 'utf8')
const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
const heading = new RegExp(`^##\\s+(?:\\[)?v?${escaped}(?:\\])?[^\\r\\n]*\\r?\\n`, 'm')
const match = heading.exec(changelog)
if (!match) throw new Error(`CHANGELOG.md 缺少 v${version} 的版本段落`)

const remainder = changelog.slice(match.index + match[0].length)
const nextHeading = remainder.search(/^##\s+/m)
const section = (nextHeading >= 0 ? remainder.slice(0, nextHeading) : remainder).trim()
if (!section) throw new Error(`v${version} 的 CHANGELOG 段落为空`)

const notes = [
  `# 蜡笔小新 v${version}`,
  '',
  section,
  '',
  '## 安装',
  '',
  ...(rust ? [
    `- CrayonShinChan-${version}-rust-x64.exe：Rust NSIS 安装版`,
    `- CrayonShinChan-${version}-rust-x64.zip：Rust 免安装版`,
    '- 使用 SHA256SUMS-rust.txt 核对文件完整性。包内包含完整 Spider 运行时。',
    '- 需要系统 Microsoft Edge WebView2 Runtime；不需要安装 Rust、Node.js 或 Java。',
    '- 默认安装到当前用户 Programs/crayon-shin-chan-rust，旧版数据请退出程序后复制到新目录。卸载保留 data。'
  ] : [
    `- CrayonShinChan-${version}-x64.exe：NSIS 安装版`,
    `- CrayonShinChan-${version}-x64.zip：免安装版`,
    `- CrayonShinChan-${version}-x64-lite.exe：精简版（不含 Spider 运行时）`,
    `- CrayonShinChan-${version}-x64-lite.zip：精简免安装版`,
    '- 完整版用 SHA256SUMS.txt、精简版用 SHA256SUMS-lite.txt 核对文件完整性'
  ]),
  '',
  '程序没有代码签名证书，首次运行可能出现 Windows SmartScreen 提示。'
].join('\n')

const directory = join(root, 'release', ...(rust ? ['rust'] : []))
const output = join(directory, 'RELEASE_NOTES.md')
if (args.includes('--write')) {
  await fs.mkdir(directory, { recursive: true })
  await fs.writeFile(output, `${notes}\n`, 'utf8')
  console.log(`[release-notes] 已写入 ${output}`)
} else {
  process.stdout.write(`${notes}\n`)
}
