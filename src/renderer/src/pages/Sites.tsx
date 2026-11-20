import { useEffect, useMemo, useState } from 'react'
import type {
  AuditResult,
  Site,
  SiteType,
  SpiderCacheInfo,
  SpiderFailureKind,
  SpiderProbeResult,
  SpiderRuntimeStatus
} from '@shared/types'
import type { DetectResult } from '@core/source/detect'
import type { HealthSummary } from '@core/health/score'
import { SITE_TYPE_LABEL, SITE_TYPE_TAG } from '@shared/siteLabels'
import { Switch } from '@/components/Switch'
import { EmptyState } from '@/components/EmptyState'
import { Icon } from '@/components/icons'
import { useApp } from '@/store/useApp'

const TYPE_ORDER: SiteType[] = [3, 4, 1, 0, 2]

const TYPE_OPTIONS: { value: SiteType; label: string }[] = [
  { value: 3, label: '苹果CMS' },
  { value: 4, label: 'JSON 接口' },
  { value: 1, label: '网页规则' }
]

const SPIDER_FAILURE_LABEL: Record<SpiderFailureKind, string> = {
  missing_class: '缺类',
  wrong_type: '类型不对',
  android_api: 'Android API',
  native: 'native 加固',
  timeout: '超时',
  tls: 'TLS/证书',
  empty_home: '首页为空',
  empty_search: '搜索为空',
  empty_play_fields: '空播放字段',
  parse_required: '需解析',
  http: 'HTTP/网络',
  unknown: '未知'
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`
}

function detectHint(detected: DetectResult | null): string {
  if (!detected) return ''
  switch (detected.kind) {
    case 'cms':
      return `识别为苹果CMS 接口${detected.name ? `（${detected.name}）` : ''}`
    case 'tvbox':
      return `识别为 TVBox 配置（含 ${detected.siteCount ?? 0} 个站点），请到「设置 → 配置源」添加`
    case 'multirepo':
      return `识别为 TVBox 多仓清单（${detected.repoCount ?? 0} 个仓库），请到「设置 → 配置源」添加`
    case 'm3u':
      return '识别为直播源，请到「设置 → 配置源」添加'
    case 'encrypted':
      return '识别为加密 TVBox 配置，请到「设置 → 配置源」添加（会自动解密）'
    case 'image':
      return '识别为图片伪装配置，请到「设置 → 配置源」添加（会自动解包）'
    case 'html':
      return '识别为网页，可作为网页规则源添加'
    default:
      return '未能识别类型，请手动选择'
  }
}

function canAddAsSingle(detected: DetectResult | null): boolean {
  if (!detected) return true
  return detected.kind === 'cms' || detected.kind === 'html' || detected.kind === 'unknown'
}

export function Sites(): JSX.Element {
  const sites = useApp((s) => s.sites)
  const setSiteEnabled = useApp((s) => s.setSiteEnabled)
  const setSitesEnabled = useApp((s) => s.setSitesEnabled)
  const refreshSites = useApp((s) => s.refreshSites)
  const notify = useApp((s) => s.notify)

  const [kw, setKw] = useState('')
  const [typeFilter, setTypeFilter] = useState<'all' | SiteType>('all')
  const [sourceFilter, setSourceFilter] = useState<string>('all')
  const [onlyEnabled, setOnlyEnabled] = useState(false)
  const [testing, setTesting] = useState<Record<string, boolean>>({})
  const [testResults, setTestResults] = useState<
    Record<string, { ok: boolean; text: string; category?: SpiderFailureKind }>
  >({})
  const [healthMap, setHealthMap] = useState<Record<string, HealthSummary>>({})
  const [audits, setAudits] = useState<Record<string, AuditResult | 'running'>>({})
  const [spiderStatus, setSpiderStatus] = useState<SpiderRuntimeStatus | null>(null)
  const [spiderProbe, setSpiderProbe] = useState<SpiderProbeResult | 'running' | null>(null)
  const [spiderCache, setSpiderCache] = useState<SpiderCacheInfo | null>(null)

  const [showAdd, setShowAdd] = useState(false)
  const [addMode, setAddMode] = useState<'single' | 'batch'>('single')
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [type, setType] = useState<SiteType | 'auto'>('auto')
  const [detected, setDetected] = useState<DetectResult | null>(null)
  const [detecting, setDetecting] = useState(false)
  const [adding, setAdding] = useState(false)
  const [batchText, setBatchText] = useState('')

  const sourceNames = useMemo(
    () => Array.from(new Set(sites.map((s) => s.sourceName))).sort(),
    [sites]
  )

  const typeCounts = useMemo(() => {
    const map = new Map<SiteType, number>()
    for (const s of sites) map.set(s.type, (map.get(s.type) ?? 0) + 1)
    return map
  }, [sites])

  const filtered = useMemo(() => {
    const k = kw.trim().toLowerCase()
    return sites.filter((s) => {
      if (typeFilter !== 'all' && s.type !== typeFilter) return false
      if (sourceFilter !== 'all' && s.sourceName !== sourceFilter) return false
      if (onlyEnabled && !s.enabled) return false
      if (!k) return true
      return (
        s.name.toLowerCase().includes(k) ||
        s.key.toLowerCase().includes(k) ||
        s.api.toLowerCase().includes(k)
      )
    })
  }, [sites, kw, typeFilter, sourceFilter, onlyEnabled])

  useEffect(() => {
    void window.api.health
      .list()
      .then(setHealthMap)
      .catch(() => undefined)
  }, [sites])

  useEffect(() => {
    void window.api.spider
      .status()
      .then(setSpiderStatus)
      .catch(() => undefined)
    void window.api.spider
      .cache()
      .then(setSpiderCache)
      .catch(() => undefined)
  }, [])

  function resetAddForm(): void {
    setName('')
    setUrl('')
    setType('auto')
    setDetected(null)
    setBatchText('')
  }

  async function detect(): Promise<void> {
    const value = url.trim()
    if (!value) {
      notify('请先填写接口地址', 'warn')
      return
    }
    setDetecting(true)
    try {
      const result = await window.api.localSites.detect(value)
      setDetected(result)
      if (!name && result.name) setName(result.name)
    } catch (err) {
      setDetected(null)
      notify((err as Error).message, 'danger')
    } finally {
      setDetecting(false)
    }
  }

  async function addOne(): Promise<void> {
    const value = url.trim()
    if (!value) {
      notify('请先填写接口地址', 'warn')
      return
    }
    if (!canAddAsSingle(detected)) {
      notify(detectHint(detected), 'warn')
      return
    }
    setAdding(true)
    try {
      const resolvedType: SiteType | undefined =
        type === 'auto'
          ? detected?.kind === 'cms'
            ? 3
            : detected?.kind === 'html'
              ? 1
              : undefined
          : type
      await window.api.localSites.add({
        name: name.trim() || undefined,
        api: value,
        type: resolvedType
      })
      await refreshSites()
      resetAddForm()
      setShowAdd(false)
      notify('已添加影视源', 'success')
    } catch (err) {
      notify((err as Error).message, 'danger')
    } finally {
      setAdding(false)
    }
  }

  async function addBatch(): Promise<void> {
    const urls = batchText
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
    if (!urls.length) {
      notify('请先粘贴至少一个地址', 'warn')
      return
    }
    setAdding(true)
    let ok = 0
    let fail = 0
    let skipped = 0
    try {
      for (const item of urls) {
        try {
          const result = await window.api.localSites.detect(item)
          if (result.kind === 'tvbox' || result.kind === 'multirepo' || result.kind === 'm3u') {
            skipped++
            continue
          }
          const resolvedType: SiteType | undefined =
            result.kind === 'cms' ? 3 : result.kind === 'html' ? 1 : undefined
          await window.api.localSites.add({ api: item, type: resolvedType })
          ok++
        } catch {
          fail++
        }
      }
      await refreshSites()
      notify(
        `批量导入完成：成功 ${ok}，失败 ${fail}${skipped ? `，跳过 ${skipped} 个非单源配置` : ''}`,
        fail || skipped ? 'warn' : 'success'
      )
      if (ok) {
        resetAddForm()
        setShowAdd(false)
      }
    } finally {
      setAdding(false)
    }
  }

  async function test(site: Site): Promise<void> {
    setTesting((t) => ({ ...t, [site.key]: true }))
    try {
      const result = await window.api.sites.test(site.key)
      setTestResults((h) => ({
        ...h,
        [site.key]: {
          ok: result.ok,
          text: result.ok
            ? `${result.ms}ms · ${result.message}`
            : `${result.category ? `${SPIDER_FAILURE_LABEL[result.category]} · ` : ''}${result.message}`,
          category: result.category
        }
      }))
      if (!result.ok) {
        const label = result.category ? `${SPIDER_FAILURE_LABEL[result.category]} · ` : ''
        notify(`${site.name}：${label}${result.message}`, 'warn')
      }
    } finally {
      setTesting((t) => {
        const next = { ...t }
        delete next[site.key]
        return next
      })
    }
  }

  async function testAllVisible(): Promise<void> {
    const targets = filtered.filter((s) => !s.unsupported && s.enabled)
    for (const site of targets) await test(site)
    notify(`已测试 ${targets.length} 个站点`, 'success')
  }

  async function audit(site: Site): Promise<void> {
    setAudits((a) => ({ ...a, [site.key]: 'running' }))
    try {
      const result = await window.api.sites.audit(site.key)
      setAudits((a) => ({ ...a, [site.key]: result }))
      setHealthMap(await window.api.health.list())
      if (!result.ok) notify(`${site.name} 审计未通过：${result.reason ?? '未知原因'}`, 'warn')
    } catch (err) {
      notify(`${site.name} 审计失败：${(err as Error).message}`, 'danger')
      setAudits((a) => {
        const next = { ...a }
        delete next[site.key]
        return next
      })
    }
  }

  async function ignore(site: Site): Promise<void> {
    if (
      !window.confirm(
        `不再显示订阅站点「${site.name}」？\n之后重新同步订阅也不会把它带回来。可在「设置 → 忽略列表」清空恢复。`
      )
    ) {
      return
    }
    await window.api.sites.ignore(site.key)
    await refreshSites()
    notify(`已忽略「${site.name}」`, 'success')
  }

  async function removeManual(site: Site): Promise<void> {
    if (!window.confirm(`确定删除手动影视源「${site.name}」吗？`)) return
    await window.api.localSites.remove(site.key)
    await refreshSites()
    notify(`已删除「${site.name}」`, 'success')
  }

  const enabledKeys = filtered.filter((s) => s.enabled).map((s) => s.key)
  const spiderSites = sites.filter((s) => Boolean(s.spiderClass))
  const disabledSpider = filtered.filter(
    (s) => Boolean(s.spiderClass) && !s.unsupported && !s.enabled
  )

  async function enableVisibleSpider(): Promise<void> {
    if (!disabledSpider.length) return
    if (
      !window.confirm(
        `将启用当前列表中的 ${disabledSpider.length} 个 Spider 站点。\n` +
          '首次搜索会按需启动 JVM 并转换 spider jar，站点越多等待越久；建议先按来源筛选。继续？'
      )
    ) {
      return
    }
    await setSitesEnabled(
      disabledSpider.map((s) => s.key),
      true
    )
    await refreshSites()
    notify(`已启用 ${disabledSpider.length} 个 Spider 站点`, 'success')
  }

  async function probeSpiderRuntime(): Promise<void> {
    setSpiderProbe('running')
    try {
      const result = await window.api.spider.probe()
      setSpiderProbe(result)
      notify(result.message, result.ok ? 'success' : 'danger')
    } catch (err) {
      const result = { ok: false, ms: 0, message: (err as Error).message }
      setSpiderProbe(result)
      notify(result.message, 'danger')
    }
  }

  async function clearSpiderCache(): Promise<void> {
    if (!window.confirm('清理 Spider 缓存会删除已下载/转换的 jar，下次使用时会重新下载并转换。继续？')) {
      return
    }
    try {
      const info = await window.api.spider.clearCache()
      setSpiderCache(info)
      notify('Spider 缓存已清理', 'success')
    } catch (err) {
      notify(`清理缓存失败：${(err as Error).message}`, 'danger')
    }
  }

  async function clearSiteCache(site: Site): Promise<void> {
    if (!window.confirm(`清理「${site.name}」的 Spider 缓存？下次使用时会重新下载并转换。`)) return
    try {
      const info = await window.api.spider.clearCache(site.key)
      setSpiderCache(info)
      notify(`已清理 ${site.name} 的 Spider 缓存`, 'success')
    } catch (err) {
      notify(`清理缓存失败：${(err as Error).message}`, 'danger')
    }
  }

  async function restartSpiderRuntime(): Promise<void> {
    setSpiderProbe('running')
    try {
      const result = await window.api.spider.restart()
      setSpiderProbe(result)
      notify(result.message, result.ok ? 'success' : 'danger')
    } catch (err) {
      const result = { ok: false, ms: 0, message: (err as Error).message }
      setSpiderProbe(result)
      notify(result.message, 'danger')
    }
  }

  if (!sites.length) {
    return (
      <div className="page">
        <div className="page-head">
          <div>
            <div className="page-title">影视源</div>
            <div className="page-sub">手动添加，或从配置源导入</div>
          </div>
          <div className="page-actions">
            <button type="button" className="btn btn-primary" onClick={() => setShowAdd(true)}>
              <Icon name="plus" size={14} /> 添加影视源
            </button>
          </div>
        </div>
        {showAdd ? (
          <AddPanel
            mode={addMode}
            setMode={setAddMode}
            name={name}
            setName={setName}
            url={url}
            setUrl={(value) => {
              setUrl(value)
              setDetected(null)
            }}
            type={type}
            setType={setType}
            detected={detected}
            detecting={detecting}
            adding={adding}
            batchText={batchText}
            setBatchText={setBatchText}
            onDetect={() => void detect()}
            onAdd={() => void addOne()}
            onBatch={() => void addBatch()}
            onClose={() => {
              setShowAdd(false)
              resetAddForm()
            }}
          />
        ) : null}
        <EmptyState
          icon={<Icon name="layers" size={26} />}
          title="还没有任何影视源"
          desc="可以直接添加单个苹果CMS / JSON 接口，也可以在「设置 → 配置源」导入 TVBox 订阅。"
        />
      </div>
    )
  }

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <div className="page-title">影视源</div>
          <div className="page-sub">
            共 {sites.length} 个站点 · 启用 {sites.filter((s) => s.enabled).length} 个 · 当前列表{' '}
            {filtered.length} 个
            {spiderSites.length
              ? ` · Spider ${spiderSites.length} 个（已启用 ${spiderSites.filter((s) => s.enabled).length}）`
              : ''}
          </div>
        </div>
        <div className="page-actions">
          <button type="button" className="btn btn-primary" onClick={() => setShowAdd((v) => !v)}>
            <Icon name="plus" size={14} /> 添加影视源
          </button>
          <button type="button" className="btn btn-sm" onClick={() => void refreshSites()}>
            <Icon name="refresh" size={14} /> 刷新列表
          </button>
          <button type="button" className="btn btn-sm" onClick={() => void testAllVisible()}>
            <Icon name="check" size={14} /> 测试当前列表
          </button>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => void setSitesEnabled(enabledKeys, false)}
            disabled={!enabledKeys.length}
          >
            全部禁用
          </button>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => void enableVisibleSpider()}
            disabled={!disabledSpider.length}
            title="只启用当前筛选结果中尚未启用的 Spider 站点"
          >
            启用可见 Spider
          </button>
        </div>
      </div>

      {spiderSites.length ? (
        <div className={`banner ${spiderStatus?.available === false ? 'banner-danger' : 'banner-info'}`}>
          <span style={{ flex: 1 }}>
            {spiderStatus?.available === false
              ? `Spider 运行时不可用：${spiderStatus.reason ?? '缺少组件'}。请重新安装完整版本。`
              : spiderStatus?.available
                ? '内置 Spider 运行时可用。Spider 站点开启后即可搜索、详情与播放；首次使用会启动 JVM 并转换 jar，稍等几秒即可。'
                : '正在检查内置 Spider 运行时…'}
            {spiderProbe && spiderProbe !== 'running' ? (
              <span className={spiderProbe.ok ? 'text-3' : ''} style={spiderProbe.ok ? undefined : { color: 'var(--danger)' }}>
                {' '}
                自检：{spiderProbe.message}（{spiderProbe.ms}ms）
              </span>
            ) : null}
            {spiderCache ? (
              <span className="text-3">
                {' '}
                · 缓存 {formatBytes(spiderCache.bytes)} / {spiderCache.files} 个文件
              </span>
            ) : null}
          </span>
          <button
            type="button"
            className="btn btn-sm"
            disabled={spiderProbe === 'running'}
            onClick={() => void probeSpiderRuntime()}
          >
            {spiderProbe === 'running' ? <span className="spin" /> : null} 运行时自检
          </button>
          <button type="button" className="btn btn-sm" onClick={() => void clearSpiderCache()}>
            <Icon name="trash" size={13} /> 清理缓存
          </button>
          <button
            type="button"
            className="btn btn-sm"
            disabled={spiderProbe === 'running'}
            onClick={() => void restartSpiderRuntime()}
          >
            <Icon name="refresh" size={13} /> 重新准备运行时
          </button>
        </div>
      ) : null}

      {showAdd ? (
        <AddPanel
          mode={addMode}
          setMode={setAddMode}
          name={name}
          setName={setName}
          url={url}
          setUrl={(value) => {
            setUrl(value)
            setDetected(null)
          }}
          type={type}
          setType={setType}
          detected={detected}
          detecting={detecting}
          adding={adding}
          batchText={batchText}
          setBatchText={setBatchText}
          onDetect={() => void detect()}
          onAdd={() => void addOne()}
          onBatch={() => void addBatch()}
          onClose={() => {
            setShowAdd(false)
            resetAddForm()
          }}
        />
      ) : null}

      <div className="filter-bar">
        <div className="search-box" style={{ width: 260 }}>
          <span className="search-icon">
            <Icon name="search" size={15} />
          </span>
          <input
            value={kw}
            placeholder="搜索站点名称 / key / 接口"
            onChange={(e) => setKw(e.target.value)}
          />
        </div>

        <div className="chips">
          <button
            type="button"
            className={`chip${typeFilter === 'all' ? ' active' : ''}`}
            onClick={() => setTypeFilter('all')}
          >
            全部类型 <span className="chip-count">{sites.length}</span>
          </button>
          {TYPE_ORDER.filter((t) => typeCounts.has(t)).map((t) => (
            <button
              key={t}
              type="button"
              className={`chip${typeFilter === t ? ' active' : ''}`}
              onClick={() => setTypeFilter(t)}
            >
              {SITE_TYPE_LABEL[t]} <span className="chip-count">{typeCounts.get(t)}</span>
            </button>
          ))}
        </div>

        <div className="grow" />

        {sourceNames.length > 1 ? (
          <select
            className="input"
            style={{ width: 170 }}
            value={sourceFilter}
            onChange={(e) => setSourceFilter(e.target.value)}
          >
            <option value="all">全部来源</option>
            {sourceNames.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        ) : null}

        <button
          type="button"
          className={`chip${onlyEnabled ? ' active' : ''}`}
          onClick={() => setOnlyEnabled((v) => !v)}
        >
          只看已启用
        </button>
      </div>

      <div className="panel">
        <div className="list">
          {filtered.map((site) => {
            const testResult = testResults[site.key]
            const health = healthMap[site.key]
            const auditResult = audits[site.key]
            return (
              <div className="list-row" key={site.key}>
                <Switch
                  checked={site.enabled}
                  disabled={Boolean(site.unsupported)}
                  onChange={(v) => void setSiteEnabled(site.key, v)}
                  title={site.unsupported ?? '启用 / 停用该站点'}
                />

                <div className="list-row-main">
                  <div className="list-row-title site-name">
                    <span className="truncate">{site.name}</span>
                    <span className={`tag ${site.spiderClass ? 'tag-success' : SITE_TYPE_TAG[site.type]}`}>
                      {site.spiderClass ? 'Spider' : SITE_TYPE_LABEL[site.type]}
                    </span>
                    {site.spiderClass && !site.enabled ? (
                      <span className="tag tag-outline">未启用</span>
                    ) : null}
                    {site.origin === 'manual' ? <span className="tag tag-accent">手动</span> : null}
                    {site.searchable ? <span className="tag tag-outline">可搜索</span> : null}
                    {!site.searchable && !site.unsupported ? (
                      <span className="tag tag-outline">仅浏览</span>
                    ) : null}
                  </div>
                  <div className="list-row-sub">
                    <span className="site-key">{site.key}</span>
                    <span style={{ margin: '0 8px' }}>·</span>
                    {site.unsupported ? (
                      <span style={{ color: 'var(--warn)' }}>{site.unsupported}</span>
                    ) : site.spiderClass ? (
                      <span>
                        <span className="mono">{site.api}</span>
                        <span style={{ margin: '0 8px' }}>·</span>
                        内置 JVM 运行，{site.enabled ? '已启用' : '开启开关后可用'}
                        {(() => {
                          const cached = spiderCache?.sites.find((item) => item.siteKey === site.key)
                          if (!cached?.cached) return <span> · 尚未缓存</span>
                          return (
                            <span className="text-3">
                              {' '}
                              · 缓存 {formatBytes(cached.bytes)} · SHA256 {cached.sha256?.slice(0, 12)}…
                            </span>
                          )
                        })()}
                      </span>
                    ) : (
                      <span className="mono">{site.api || '未配置接口'}</span>
                    )}
                  </div>
                  {auditResult ? (
                    <div className="list-row-sub" style={{ marginTop: 4 }}>
                      {auditResult === 'running' ? (
                        <span className="hstack" style={{ gap: 6 }}>
                          <span className="spin" /> 正在审计…
                        </span>
                      ) : (
                        <span className={auditResult.ok ? 'text-3 fs-12' : 'fs-12'} style={auditResult.ok ? undefined : { color: 'var(--warn)' }}>
                          {auditResult.ok
                            ? `审计通过：分类 ${auditResult.categories} · 搜索 ${auditResult.searchHits} 条 · 线路 ${auditResult.detailLines} / ${auditResult.episodes} 集 · ${auditResult.ms}ms`
                            : `审计失败（${auditResult.failStage ?? '未知'}）：${auditResult.reason ?? '未知原因'}`}
                        </span>
                      )}
                    </div>
                  ) : null}
                </div>

                {testResult ? (
                  <span className="health" title={testResult.text}>
                    <span className={`dot ${testResult.ok ? 'ok' : 'bad'}`} />
                    {testResult.text.length > 26 ? `${testResult.text.slice(0, 26)}…` : testResult.text}
                  </span>
                ) : health && health.samples > 0 ? (
                  <span
                    className="health"
                    title={`成功率 ${Math.round(health.successRate * 100)}% · 平均 ${health.avgMs}ms · 样本 ${health.samples}`}
                  >
                    <span className={`dot ${health.cooling ? 'bad' : health.successRate >= 0.5 ? 'ok' : 'bad'}`} />
                    {health.cooling
                      ? '冷却中'
                      : `${Math.round(health.successRate * 100)}% · ${health.avgMs}ms`}
                  </span>
                ) : typeof site.health === 'number' ? (
                  <span className="health">
                    <span className={`dot ${site.health >= 0 ? 'ok' : 'bad'}`} />
                    {site.health >= 0 ? `${site.health}ms` : '失败'}
                  </span>
                ) : (
                  <span className="health">未测</span>
                )}

                <div className="list-row-actions">
                  <button
                    type="button"
                    className="btn btn-sm"
                    disabled={Boolean(testing[site.key]) || Boolean(site.unsupported)}
                    onClick={() => void test(site)}
                  >
                    {testing[site.key] ? <span className="spin" /> : null} 测试
                  </button>
                  {site.spiderClass ? (
                    <button type="button" className="btn btn-sm" onClick={() => void clearSiteCache(site)}>
                      清缓存
                    </button>
                  ) : null}
                  <button
                    type="button"
                    className="btn btn-sm"
                    disabled={auditResult === 'running' || Boolean(site.unsupported)}
                    onClick={() => void audit(site)}
                  >
                    {auditResult === 'running' ? <span className="spin" /> : null} 审计
                  </button>
                  {site.origin === 'manual' ? (
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => void removeManual(site)}
                    >
                      删除
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="btn btn-sm"
                      title="不再显示该站点，后续同步也不会复活"
                      onClick={() => void ignore(site)}
                    >
                      忽略
                    </button>
                  )}
                </div>
              </div>
            )
          })}

          {!filtered.length ? (
            <EmptyState
              icon={<Icon name="search" size={26} />}
              title="没有匹配的站点"
              desc="换个关键词或清空筛选条件试试。"
            />
          ) : null}
        </div>
      </div>
    </div>
  )
}

function AddPanel({
  mode,
  setMode,
  name,
  setName,
  url,
  setUrl,
  type,
  setType,
  detected,
  detecting,
  adding,
  batchText,
  setBatchText,
  onDetect,
  onAdd,
  onBatch,
  onClose
}: {
  mode: 'single' | 'batch'
  setMode: (mode: 'single' | 'batch') => void
  name: string
  setName: (value: string) => void
  url: string
  setUrl: (value: string) => void
  type: SiteType | 'auto'
  setType: (value: SiteType | 'auto') => void
  detected: DetectResult | null
  detecting: boolean
  adding: boolean
  batchText: string
  setBatchText: (value: string) => void
  onDetect: () => void
  onAdd: () => void
  onBatch: () => void
  onClose: () => void
}): JSX.Element {
  return (
    <div className="panel" style={{ marginBottom: 14 }}>
      <div className="section-head" style={{ marginBottom: 10 }}>
        <div>
          <div className="section-title">添加影视源</div>
          <div className="section-more">
            支持苹果CMS 采集接口和网页规则站；TVBox 订阅 / 多仓请到「设置 → 配置源」添加
          </div>
        </div>
        <div className="page-actions">
          <button
            type="button"
            className={`chip${mode === 'single' ? ' active' : ''}`}
            onClick={() => setMode('single')}
          >
            单个添加
          </button>
          <button
            type="button"
            className={`chip${mode === 'batch' ? ' active' : ''}`}
            onClick={() => setMode('batch')}
          >
            批量导入
          </button>
          <button type="button" className="btn btn-sm" onClick={onClose}>
            收起
          </button>
        </div>
      </div>

      {mode === 'single' ? (
        <>
          <div className="form-row">
            <div className="field" style={{ flex: '0 0 200px' }}>
              <label>名称（可选）</label>
              <input
                className="input"
                value={name}
                placeholder="留空则用域名"
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="field grow">
              <label>接口地址</label>
              <input
                className="input"
                value={url}
                placeholder="https://example.com/api.php/provide/vod/"
                onChange={(e) => setUrl(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') onDetect()
                }}
              />
            </div>
            <div className="field" style={{ flex: '0 0 150px' }}>
              <label>类型</label>
              <select
                className="input"
                value={type}
                onChange={(e) => setType(e.target.value === 'auto' ? 'auto' : (Number(e.target.value) as SiteType))}
              >
                <option value="auto">自动判断</option>
                {TYPE_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <div className="form-row" style={{ alignItems: 'center' }}>
            <button type="button" className="btn" onClick={onDetect} disabled={detecting || adding}>
              {detecting ? <span className="spin" /> : null} 探测类型
            </button>
            <button
              type="button"
              className="btn btn-primary"
              onClick={onAdd}
              disabled={adding || !canAddAsSingle(detected)}
            >
              {adding ? <span className="spin" /> : null} 添加
            </button>
            {detected ? (
              <span className="text-3 fs-13" style={{ marginLeft: 4 }}>
                {detectHint(detected)}
              </span>
            ) : (
              <span className="text-3 fs-13" style={{ marginLeft: 4 }}>
                先探测一下可以确认接口类型
              </span>
            )}
          </div>
        </>
      ) : (
        <>
          <div className="field">
            <label>每行一个接口地址</label>
            <textarea
              className="input"
              rows={6}
              value={batchText}
              placeholder={'https://a.com/api.php/provide/vod/\nhttps://b.com/api.php/provide/vod/'}
              onChange={(e) => setBatchText(e.target.value)}
              style={{ resize: 'vertical', fontFamily: 'var(--mono, monospace)' }}
            />
          </div>
          <div className="form-row" style={{ marginTop: 10 }}>
            <button type="button" className="btn btn-primary" onClick={onBatch} disabled={adding}>
              {adding ? <span className="spin" /> : null} 开始导入
            </button>
            <span className="text-3 fs-13">会自动探测每个地址；TVBox 配置与直播源会被跳过</span>
          </div>
        </>
      )}
    </div>
  )
}
