// Rust owns resume/retry/history decisions; adapters only translate media APIs.
window.__crayonCreatePlayer = function createPlayer(cfg) {
  // Rust initiates retries; ArtPlayer reconnects would leave a second transmuxer attached.
  window.Artplayer.RECONNECT_TIME_MAX = 0
  window.mpegts?.LoggingControl.applyConfig({ enableAll: false, enableDebug: false, enableVerbose: false, enableInfo: false, enableWarn: false, enableError: false })
  const container = document.getElementById(cfg.id)
  if (!container || !container.isConnected) throw new Error('找不到播放器容器')
  container.innerHTML = ''
  const players = window.__crayonPlayers ||= new Map()
  players.get(cfg.session)?.destroy(false)
  window.__crayonActiveSession = cfg.session
  let art
  let disposed = false
  const cleanups = []
  const current = () => !disposed && container.isConnected && window.__crayonActiveSession === cfg.session && (!art || players.get(cfg.session) === art)
  const emit = (event, extra = {}, instance = art) => {
    if (!current() && !(event === 'destroy' && !disposed && players.get(cfg.session) === art)) return
    window.__crayonSend?.({ session: cfg.session, event,
      position: instance?.currentTime || 0, duration: Number.isFinite(instance?.duration) ? instance.duration : 0,
      capturing: !!instance?.__capture, ...extra })
  }
  const fail = (category, message, engine = cfg.kind) => emit('error', { error: message, errorCategory: category, engine })
  const dispose = () => {
    if (disposed) return
    emit('destroy')
    disposed = true
    if (players.get(cfg.session) === art) players.delete(cfg.session)
    for (const cleanup of cleanups.reverse()) {
      try {
        Promise.resolve(cleanup()).catch(() => { if (art) art.__cleanupError = '播放器资源释放失败' })
      } catch { if (art) art.__cleanupError = '播放器资源释放失败' }
    }
  }
  const listen = (instance, name, callback) => {
    instance.on(name, callback)
    cleanups.push(() => instance.off(name, callback))
  }
  const dashError = (error, engine) => {
    if (!current() || error?.code === window.shaka.util.Error.Code.OPERATION_ABORTED) return
    const category = error?.category
    if (category === 1) fail('network', 'DASH 网络请求失败，请重试或换线', engine)
    else if (category === 3 || category === 5) fail('decode', '当前环境无法解码这条 DASH 的音视频编码，请使用外部播放器', engine)
    else if (category === 6) fail('drm', '这条 DASH 需要受保护内容授权，当前播放器无法播放', engine)
    else fail('format', 'DASH 清单或分片格式无效，请换线', engine)
  }
  const loadDash = async (video, url, instance) => {
    const shaka = window.shaka
    shaka?.polyfill.installAll()
    if (!shaka?.Player.isBrowserSupported()) return fail('capability', '当前环境不支持 DASH 播放，请使用外部播放器')
    const dash = new shaka.Player()
    instance.dash = dash
    const network = dash.getNetworkingEngine()
    const endpoint = new URL(url)
    endpoint.search = ''
    endpoint.hash = ''
    const unwrap = (uri) => {
      const parsed = new URL(uri)
      return parsed.origin === endpoint.origin && parsed.pathname === endpoint.pathname
        ? parsed.searchParams.get('url') || uri : uri
    }
    const requestFilter = (_, request) => {
      request.uris = request.uris.map((uri) => {
        const target = new URL(unwrap(uri), cfg.sourceUrl)
        if (!['http:', 'https:'].includes(target.protocol)) throw new Error('DASH 媒体协议不受支持')
        const proxy = new URL(endpoint)
        proxy.searchParams.set('url', target.href)
        return proxy.href
      })
    }
    const responseFilter = (_, response) => {
      // DASH resolves SegmentTemplate/BaseURL against response.uri, including redirects.
      const finalUrl = response.headers?.['x-crayon-media-url']
      response.uri = finalUrl ? decodeURIComponent(finalUrl) : unwrap(response.uri)
      response.originalUri = unwrap(response.originalUri || response.uri)
    }
    network.registerRequestFilter(requestFilter)
    network.registerResponseFilter(responseFilter)
    const error = (event) => {
      const detail = event.detail
      if (detail?.severity === shaka.util.Error.Severity.CRITICAL) dashError(detail, 'shaka')
    }
    dash.addEventListener('error', error)
    const label = (track, index) => String(track.label || track.language || `轨道 ${index + 1}`)
    const escaped = (text) => String(text).replace(/[&<>"']/g, (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[char]))
    const tracks = () => {
      if (!current()) return
      const update = (name, title, list, select, off) => {
        if (!list.length) { if (instance.setting.find(name)) instance.setting.remove(name); return }
        const selector = list.map((track, index) => ({ html: escaped(label(track, index)), default: !!track.active, track }))
        if (off) selector.unshift({ html: '关闭', track: null, default: !list.some((track) => track.active) })
        const setting = { name, html: title, selector, onSelect: (item) => { if (current()) select(item.track); return item.html } }
        instance.setting[instance.setting.find(name) ? 'update' : 'add'](setting)
      }
      update('crayon-audio', '音轨', dash.getAudioTracks(), (track) => dash.selectAudioTrack(track))
      update('crayon-subtitle', '字幕', dash.getTextTracks(), (track) => dash.selectTextTrack(track), true)
    }
    dash.addEventListener('trackschanged', tracks)
    cleanups.push(() => {
      dash.removeEventListener('error', error)
      dash.removeEventListener('trackschanged', tracks)
      network.unregisterRequestFilter(requestFilter)
      network.unregisterResponseFilter(responseFilter)
      return dash.destroy()
    })
    const retry = { maxAttempts: 3, baseDelay: 700, timeout: 20000, connectionTimeout: 10000, stallTimeout: 10000 }
    dash.configure({ manifest: { retryParameters: retry }, streaming: { retryParameters: retry, failureCallback: (failure) => dashError(failure, 'shaka') } })
    try {
      await dash.attach(video)
      if (!current()) return
      await dash.load(url, undefined, 'application/dash+xml')
      if (current()) tracks()
    } catch (failure) { dashError(failure, 'shaka') }
  }
  const custom = (video, url, instance) => {
    const start = async () => {
      if (!current()) return
      if (cfg.kind === 'html') return fail('format', '地址返回的是网页，请重新解析或换线')
      if (cfg.kind === 'hls') {
        const Hls = window.Hls
        if (Hls?.isSupported()) {
          const hls = new Hls({ maxBufferLength: 30, manifestLoadingTimeOut: 15000, abrEwmaDefaultEstimate: 4000000, manifestLoadingMaxRetry: 3, levelLoadingMaxRetry: 4, fragLoadingMaxRetry: 4 })
          instance.hls = hls
          const levels = (_, data) => emit('levels', { levels: data.levels.map(({ height, bitrate }) => ({ height, bitrate })) }, instance)
          const error = (_, data) => emit('hlsError', { fatal: data.fatal, errorType: data.type, error: data.details }, instance)
          hls.on(Hls.Events.MANIFEST_PARSED, levels)
          hls.on(Hls.Events.ERROR, error)
          cleanups.push(() => { hls.off(Hls.Events.MANIFEST_PARSED, levels); hls.off(Hls.Events.ERROR, error); hls.destroy() })
          hls.loadSource(url)
          hls.attachMedia(video)
        } else if (video.canPlayType('application/vnd.apple.mpegurl')) video.src = url
        else fail('capability', '当前环境不支持 HLS 播放')
      } else if (cfg.kind === 'flv' || cfg.kind === 'ts') {
        const mpegts = window.mpegts
        const features = mpegts?.getFeatureList()
        if (!features?.[cfg.live ? 'mseLivePlayback' : 'msePlayback']) return fail('capability', '当前环境无法转封装 FLV／MPEG-TS，请使用外部播放器')
        const player = mpegts.createPlayer({ type: cfg.kind === 'flv' ? 'flv' : 'mpegts', url, isLive: !!cfg.live, cors: true }, { enableWorker: false, enableStashBuffer: !cfg.live, autoCleanupSourceBuffer: true, autoCleanupMaxBackwardDuration: 60, autoCleanupMinBackwardDuration: 30 })
        instance.mpegts = player
        const error = (type, detail) => {
          if (type === mpegts.ErrorTypes.NETWORK_ERROR) fail('network', 'FLV／MPEG-TS 网络请求失败，请重试或换线')
          else if (detail === mpegts.ErrorDetails.MEDIA_CODEC_UNSUPPORTED || detail === mpegts.ErrorDetails.MEDIA_MSE_ERROR) fail('decode', '当前环境无法解码这条 FLV／MPEG-TS 的音视频编码，请使用外部播放器')
          else fail('format', 'FLV／MPEG-TS 数据格式无效或不受支持，请换线')
        }
        player.on(mpegts.Events.ERROR, error)
        cleanups.push(() => player.destroy())
        cleanups.push(() => player.detachMediaElement())
        cleanups.push(() => player.unload())
        cleanups.push(() => player.off(mpegts.Events.ERROR, error))
        player.attachMediaElement(video)
        player.load()
      } else if (cfg.kind === 'dash') await loadDash(video, url, instance)
      else video.src = url
    }
    Promise.resolve().then(start).catch(() => { if (current()) fail('format', '播放器无法加载这条媒体，请重试或使用外部播放器') })
  }
  art = new window.Artplayer({ container, url: cfg.url, type: 'crayon', customType: { crayon: custom }, isLive: !!cfg.live, autoplay: true, volume: 0.8, pip: true, fullscreen: true, fullscreenWeb: true, setting: true, hotkey: true, playbackRate: !cfg.live, aspectRatio: true, miniProgressBar: true, airplay: true, theme: cfg.accent || '#2563EB', lang: 'zh-CN', moreVideoAttr: { crossOrigin: 'anonymous' } })
  art.__emit = emit
  players.set(cfg.session, art)
  // ArtPlayer resets the element before its destroy event, so snapshot progress first.
  const destroy = art.destroy.bind(art)
  art.destroy = (...args) => { if (!disposed) { dispose(); destroy(...args) } }
  for (const event of ['loadedmetadata', 'durationchange', 'canplay', 'playing', 'timeupdate', 'pause', 'ended']) listen(art, `video:${event}`, () => emit(event))
  listen(art, 'video:error', () => {
    const code = art.video.error?.code
    fail(code === 2 ? 'network' : code === 3 ? 'decode' : 'format', code === 2 ? '媒体网络请求失败，请重试或换线' : code === 3 ? '当前环境无法解码此音视频，请使用外部播放器' : '媒体格式无法播放，请换线或使用外部播放器')
  })
  art.on('destroy', dispose)
  return art
}
