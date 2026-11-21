import { spawnSync } from 'node:child_process'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const script = fileURLToPath(import.meta.url)
const root = join(dirname(script), '..')
const check = process.argv.includes('--check')

// 复用项目已有的 Electron 图像解码与缩放，避免另装图像工具。
if (!process.versions.electron) {
  const env = { ...process.env }
  delete env.ELECTRON_RUN_AS_NODE
  const result = spawnSync(require('electron'), [script, ...process.argv.slice(2)], {
    env, stdio: 'inherit', windowsHide: true
  })
  if (result.error) throw result.error
  process.exit(result.status ?? 1)
}

const { app, nativeImage } = require('electron')
try {
  const source = nativeImage.createFromPath(join(root, 'build', 'icon-source.png'))
  const { width, height } = source.getSize()
  if (source.isEmpty() || width !== height || width < 512) {
    throw new Error('build/icon-source.png 必须是至少 512×512 的正方形图片')
  }

  // 沿插画已有的圆角内缘裁掉外层白框；保留画内原有的白色细节。
  const bitmap = source.toBitmap()
  const half = width / 2
  const inset = width * 0.017
  const radius = width * 0.24
  const straight = half - inset - radius
  for (let y = 0; y < height; y++) {
    const dy = Math.abs(y + 0.5 - half) - straight
    for (let x = 0; x < width; x++) {
      const dx = Math.abs(x + 0.5 - half) - straight
      const distance = Math.hypot(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0)
      const coverage = Math.min(1, Math.max(0, radius - distance + 0.5))
      if (coverage === 1) continue
      const pixel = (y * width + x) * 4
      // Electron 位图使用预乘透明度，颜色和 alpha 必须一起缩放。
      for (let channel = 0; channel < 4; channel++) {
        bitmap[pixel + channel] = Math.round(bitmap[pixel + channel] * coverage)
      }
    }
  }
  const rounded = nativeImage.createFromBitmap(bitmap, { width, height })
  const png = (size) => rounded.resize({ width: size, height: size, quality: 'best' }).toPNG()
  const sizes = [16, 24, 32, 48, 64, 128, 256]
  const blobs = sizes.map(png)
  const header = Buffer.alloc(6)
  header.writeUInt16LE(1, 2)
  header.writeUInt16LE(sizes.length, 4)
  const entries = Buffer.alloc(16 * sizes.length)
  let offset = header.length + entries.length
  sizes.forEach((size, index) => {
    const start = index * 16
    entries[start] = size === 256 ? 0 : size
    entries[start + 1] = size === 256 ? 0 : size
    entries.writeUInt16LE(1, start + 4)
    entries.writeUInt16LE(32, start + 6)
    entries.writeUInt32LE(blobs[index].length, start + 8)
    entries.writeUInt32LE(offset, start + 12)
    offset += blobs[index].length
  })

  const outputs = [
    ['build/icon.png', png(512)],
    ['build/icon.ico', Buffer.concat([header, entries, ...blobs])],
    ['src/renderer/public/icon.png', png(256)]
  ]
  for (const [name, bytes] of outputs) {
    const path = join(root, name)
    if (check) {
      if (!readFileSync(path).equals(bytes)) throw new Error(name + ' 与生成结果不一致，请运行 pnpm icon:build')
    } else {
      mkdirSync(dirname(path), { recursive: true })
      writeFileSync(path, bytes)
    }
    console.log('[PASS] ' + name + ' (' + bytes.length + ' bytes)' + (check ? ' 已同步' : ' 已生成'))
  }
  app.exit(0)
} catch (error) {
  console.error(error)
  app.exit(1)
}
