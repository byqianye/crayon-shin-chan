/**
 * 兼容旧导入路径：归一化逻辑已迁到 src/core/dedupe/normalize.ts，
 * 这样主进程、渲染进程和未来的其他壳都能复用同一套规则。
 */
export * from '@core/dedupe/normalize'
