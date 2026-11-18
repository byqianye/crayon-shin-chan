import type { SiteHealth } from '@shared/types'

const WINDOW = 20
const MAX_LATENCY_MS = 3000
/** 连续失败多少次进入冷却 */
export const COOLDOWN_FAILS = 3
/** 冷却时长 */
export const COOLDOWN_MS = 10 * 60 * 1000

export interface HealthSummary {
  score: number
  successRate: number
  avgMs: number
  cooling: boolean
  samples: number
}

/** 0–1 的健康分；没有样本时给 0.5，避免新源被永久压底 */
export function scoreHealth(health?: SiteHealth): number {
  return summarizeHealth(health).score
}

export function isCoolingDown(health?: SiteHealth): boolean {
  return Boolean(health?.cooldownUntil && health.cooldownUntil > Date.now())
}

export function summarizeHealth(health?: SiteHealth): HealthSummary {
  const samples = (health?.samples ?? []).slice(-WINDOW)
  if (!samples.length) {
    return { score: 0.5, successRate: 0.5, avgMs: 0, cooling: isCoolingDown(health), samples: 0 }
  }

  const okSamples = samples.filter((s) => s.ok)
  const successRate = okSamples.length / samples.length
  const avgMs = okSamples.length
    ? okSamples.reduce((sum, s) => sum + s.ms, 0) / okSamples.length
    : MAX_LATENCY_MS
  const latencyScore = 1 - Math.min(avgMs, MAX_LATENCY_MS) / MAX_LATENCY_MS
  const lastAt = samples[samples.length - 1]?.at ?? 0
  const freshness = Math.max(0, 1 - (Date.now() - lastAt) / (60 * 60 * 1000))
  const score = 0.6 * successRate + 0.3 * latencyScore + 0.1 * freshness

  return {
    score: Math.max(0, Math.min(1, score)),
    successRate,
    avgMs: Math.round(avgMs),
    cooling: isCoolingDown(health),
    samples: samples.length
  }
}
