import type { Site } from '@shared/types'
import { matchHeaderRules } from './http'
import { getSettings } from './settings'
import { getParsed, listSources } from './sources'

/** 配置源请求头与站点覆盖在此合并，网络适配层无需反向依赖站点 CRUD。 */
export async function resolveHeader(site: Site, url: string): Promise<Record<string, string>> {
  const sources = await listSources()
  const rules = sources.filter((source) => source.enabled).flatMap((source) => getParsed(source.id)?.headers ?? [])
  const settings = await getSettings()
  return { 'User-Agent': settings.userAgent, ...matchHeaderRules(url, rules), ...(site.header ?? {}) }
}
