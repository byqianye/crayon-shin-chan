import type { SiteHealth } from '@shared/types'
import { COOLDOWN_FAILS, COOLDOWN_MS, summarizeHealth } from '@core/health/score'
import { readStore, writeStore } from './store'

const NAME = 'health'
const MAX_SAMPLES = 50

type HealthMap = Record<string, SiteHealth>

async function readAll(): Promise<HealthMap> {
  const data = await readStore<HealthMap>(NAME, {})
  return data && typeof data === 'object' ? data : {}
}

export async function getHealthMap(): Promise<HealthMap> {
  return readAll()
}

export async function getSiteHealth(siteKey: string): Promise<SiteHealth | undefined> {
  return (await readAll())[siteKey]
}

/** 每次搜索/详情/审计完成后记一笔，用于排序与熔断 */
export async function recordSiteResult(
  siteKey: string,
  ok: boolean,
  ms: number
): Promise<SiteHealth> {
  const all = await readAll()
  const now = Date.now()
  const health: SiteHealth = all[siteKey] ?? {
    siteKey,
    samples: [],
    consecutiveFails: 0
  }
  health.samples.push({ ok, ms: Math.max(0, Math.round(ms)), at: now })
  if (health.samples.length > MAX_SAMPLES) health.samples = health.samples.slice(-MAX_SAMPLES)

  if (ok) {
    health.consecutiveFails = 0
    health.lastOkAt = now
    health.cooldownUntil = undefined
  } else {
    health.consecutiveFails += 1
    health.lastFailAt = now
    if (health.consecutiveFails >= COOLDOWN_FAILS) {
      health.cooldownUntil = now + COOLDOWN_MS
    }
  }

  all[siteKey] = health
  writeStore(NAME, all)
  return health
}

export async function clearHealth(): Promise<void> {
  writeStore(NAME, {})
}

/** 给界面用的摘要，避免把 50 条原始样本全发到渲染进程 */
export async function getHealthSummary(): Promise<
  Record<string, ReturnType<typeof summarizeHealth>>
> {
  const all = await readAll()
  return Object.fromEntries(
    Object.entries(all).map(([key, health]) => [key, summarizeHealth(health)])
  )
}
