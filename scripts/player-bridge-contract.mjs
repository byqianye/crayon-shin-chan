import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'

const source = await readFile(new URL('../native-rust/src/ui/player_bridge.js', import.meta.url), 'utf8')

const flush = async () => {
  for (let i = 0; i < 6; i++) await Promise.resolve()
}

function harness({ pendingAttach = false, supportedHls = false } = {}) {
  const events = []
  const engines = []
  const players = []
  const container = { isConnected: true, innerHTML: '' }
  let finishAttach
  const attach = pendingAttach ? new Promise((resolve) => { finishAttach = resolve }) : Promise.resolve()
  class Artplayer {
    constructor(options) {
      this.options = options
      this.listeners = new Map()
      this.video = { src: '', error: null, canPlayType: () => '', currentTime: 0, duration: 120 }
      this.currentTime = 0
      this.duration = 120
      this.notice = {}
      this.settings = new Map()
      this.setting = {
        find: (name) => this.settings.get(name),
        add: (value) => this.settings.set(value.name, value),
        update: (value) => this.settings.set(value.name, value),
        remove: (name) => this.settings.delete(name),
      }
      players.push(this)
      queueMicrotask(() => options.customType[options.type](this.video, options.url, this))
    }
    on(name, callback) {
      if (!this.listeners.has(name)) this.listeners.set(name, new Set())
      this.listeners.get(name).add(callback)
      return this
    }
    off(name, callback) { this.listeners.get(name)?.delete(callback) }
    emit(name, ...args) { for (const callback of [...(this.listeners.get(name) || [])]) callback(...args) }
    destroy() { this.currentTime = 0; this.duration = 0; this.emit('destroy'); this.isDestroy = true }
  }
  class Mpegts {
    constructor(data, options) {
      this.data = data
      this.options = options
      this.listeners = new Map()
      this.calls = []
      engines.push(this)
    }
    on(name, callback) { this.listeners.set(name, callback) }
    off(name) { this.listeners.delete(name) }
    attachMediaElement(video) { this.video = video; this.calls.push('attach') }
    load() { this.calls.push('load') }
    unload() { this.calls.push('unload') }
    detachMediaElement() { this.calls.push('detach') }
    destroy() { this.calls.push('destroy') }
  }
  class Dash {
    static isBrowserSupported() { return true }
    constructor() {
      this.listeners = new Map()
      this.calls = []
      this.network = {
        registerRequestFilter: (filter) => { this.requestFilter = filter },
        registerResponseFilter: (filter) => { this.responseFilter = filter },
        unregisterRequestFilter: () => { this.requestFilter = null },
        unregisterResponseFilter: () => { this.responseFilter = null },
      }
      engines.push(this)
    }
    addEventListener(name, callback) { this.listeners.set(name, callback) }
    removeEventListener(name) { this.listeners.delete(name) }
    getNetworkingEngine() { return this.network }
    configure(value) { this.config = value }
    attach(video) { this.video = video; this.calls.push('attach'); return attach }
    load(...args) { this.loadArguments = args; this.calls.push('load'); return Promise.resolve() }
    getAudioTracks() { return [{ language: 'zh', label: '中文', active: true }, { language: 'en', label: 'English' }] }
    getTextTracks() { return [{ id: 1, language: 'zh', active: true }] }
    selectAudioTrack(track) { this.selectedAudio = track }
    selectTextTrack(track) { this.selectedText = track }
    destroy() { this.calls.push('destroy'); return Promise.resolve() }
  }
  class Hls {
    static isSupported() { return supportedHls }
    static Events = { MANIFEST_PARSED: 'manifest', ERROR: 'error' }
    constructor() { this.listeners = new Map(); this.calls = []; engines.push(this) }
    on(name, callback) { this.listeners.set(name, callback) }
    off(name) { this.listeners.delete(name) }
    loadSource(url) { this.url = url }
    attachMedia(video) { this.video = video }
    startLoad() { this.calls.push('startLoad') }
    recoverMediaError() { this.calls.push('recoverMediaError') }
    destroy() { this.calls.push('destroy') }
  }
  const context = {
    URL, Promise, queueMicrotask, encodeURIComponent, decodeURIComponent,
    document: { getElementById: () => container, createElement: () => ({ textContent: '', innerHTML: '' }) },
    Artplayer, Hls,
    mpegts: {
      isSupported: () => true,
      getFeatureList: () => ({ mseLivePlayback: true, msePlayback: true }),
      createPlayer: (data, options) => new Mpegts(data, options),
      Events: { ERROR: 'error' },
      ErrorTypes: { NETWORK_ERROR: 'NetworkError' },
      ErrorDetails: { MEDIA_CODEC_UNSUPPORTED: 'MediaCodecUnsupported', MEDIA_FORMAT_UNSUPPORTED: 'MediaFormatUnsupported', MEDIA_FORMAT_ERROR: 'MediaFormatError' },
      LoggingControl: { applyConfig() {} },
    },
    shaka: {
      Player: Dash, polyfill: { installAll() {} },
      util: { Error: { Severity: { CRITICAL: 2 }, Category: { NETWORK: 1, MEDIA: 3, MANIFEST: 4, DRM: 6 }, Code: { OPERATION_ABORTED: 7001 } } },
    },
    __crayonPlayers: new Map(),
    __crayonSend: (event) => events.push(event),
  }
  context.window = context
  vm.runInNewContext(source, context, { filename: 'player_bridge.js' })
  const create = (kind, session = 'session') => context.__crayonCreatePlayer({
    session, id: `player-${session}`, kind, live: false,
    url: 'http://127.0.0.1:12345/token/media?url=https%3A%2F%2Ffixture.invalid%2Fwatch',
    sourceUrl: 'https://fixture.invalid/watch',
  })
  return { create, context, events, engines, players, container, finishAttach }
}

test('detected FLV without an extension reaches mpegts and keeps playback events', async () => {
  const h = harness()
  const art = await h.create('flv')
  await flush()
  assert.equal(h.engines[0].data.type, 'flv')
  assert.equal(h.engines[0].video, art.video)
  assert.ok(h.engines[0].calls.includes('load'))
  art.currentTime = 31
  art.emit('video:timeupdate')
  assert.equal(h.events.at(-1).position, 31)
  assert.equal(h.events.at(-1).session, 'session')
  assert.equal(art.options.volume, 0.8)
  art.destroy(false)
})

test('raw MPEG-TS uses transmuxing, while MP4 and unknown use the native element', async () => {
  const ts = harness()
  const art = await ts.create('ts')
  await flush()
  assert.equal(ts.engines[0].data.type, 'mpegts')
  art.destroy(false)
  for (const kind of ['mp4', 'webm', 'mkv', 'audio', 'unknown']) {
    const h = harness()
    const native = await h.create(kind)
    await flush()
    assert.equal(h.engines.length, 0)
    assert.equal(native.video.src, native.options.url)
    native.destroy(false)
  }
})

test('HLS keeps level and recovery events and prevents callbacks after replacement', async () => {
  const h = harness({ supportedHls: true })
  const art = await h.create('hls')
  await flush()
  const engine = h.engines[0]
  engine.listeners.get('manifest')(null, { levels: [{ height: 720, bitrate: 2000000 }] })
  assert.equal(h.events.at(-1).event, 'levels')
  assert.equal(h.events.at(-1).levels[0].height, 720)
  const stale = engine.listeners.get('error')
  stale(null, { fatal: true, type: 'networkError', details: 'manifestLoadError' })
  assert.equal(h.events.at(-1).event, 'hlsError')
  assert.equal(h.events.at(-1).errorType, 'networkError')
  art.hls.startLoad()
  art.hls.recoverMediaError()
  const next = await h.create('flv')
  await flush()
  const count = h.events.length
  stale(null, { fatal: true, type: 'mediaError', details: 'old' })
  assert.equal(h.events.length, count)
  assert.ok(engine.calls.includes('destroy'))
  assert.equal(h.context.__crayonPlayers.get('session'), next)
  next.destroy(false)
})

test('destroy unloads the FLV adapter once and blocks its stale callback', async () => {
  const h = harness()
  const art = await h.create('flv')
  await flush()
  const engine = h.engines[0]
  const stale = engine.listeners.get('error')
  art.currentTime = 37
  art.destroy(false)
  const afterDestroy = h.events.length
  stale('NetworkError', 'private-url-must-not-be-exposed')
  art.emit('video:timeupdate')
  art.destroy(false)
  assert.equal(h.events.length, afterDestroy)
  assert.deepEqual(engine.calls.slice(-3), ['unload', 'detach', 'destroy'])
  assert.equal(engine.calls.filter((call) => call === 'destroy').length, 1)
  assert.equal(h.context.__crayonPlayers.size, 0)
  assert.equal(h.events.at(-1).event, 'destroy')
  assert.equal(h.events.at(-1).position, 37)
  assert.equal(h.events.at(-1).duration, 120)
})

test('a new session suppresses old session events before its surface unmounts', async () => {
  const h = harness()
  const old = await h.create('flv', 'old')
  await flush()
  const stale = h.engines[0].listeners.get('error')
  const next = await h.create('dash', 'next')
  await flush()
  const count = h.events.length
  stale('NetworkError', 'old')
  old.emit('video:playing')
  assert.equal(h.events.length, count)
  next.emit('video:playing')
  assert.equal(h.events.at(-1).session, 'next')
  old.destroy(false)
  next.destroy(false)
})

test('DASH keeps proxy requests and the final redirected manifest base including Range', async () => {
  const h = harness()
  const art = await h.create('dash')
  await flush()
  const dash = h.engines[0]
  const response = {
    uri: art.options.url, originalUri: art.options.url,
    headers: { 'x-crayon-media-url': encodeURIComponent('https://fixture.invalid/redirected/dash/main.mpd?token=fixture') },
  }
  dash.responseFilter(0, response)
  assert.equal(response.uri, 'https://fixture.invalid/redirected/dash/main.mpd?token=fixture')
  const request = { uris: [new URL('video/segment-1.m4s', response.uri).href], headers: { Range: 'bytes=4-19' } }
  dash.requestFilter(1, request)
  const proxied = new URL(request.uris[0])
  assert.equal(proxied.pathname, '/token/media')
  assert.equal(proxied.searchParams.get('url'), 'https://fixture.invalid/redirected/dash/video/segment-1.m4s')
  assert.equal(request.headers.Range, 'bytes=4-19')
  dash.requestFilter(1, request)
  assert.equal(new URL(request.uris[0]).searchParams.get('url'), 'https://fixture.invalid/redirected/dash/video/segment-1.m4s')
  assert.equal(dash.loadArguments[2], 'application/dash+xml')
  art.destroy(false)
})

test('a DASH attach resolving after disposal cannot start another load', async () => {
  const h = harness({ pendingAttach: true })
  const art = await h.create('dash')
  await flush()
  const dash = h.engines[0]
  art.destroy(false)
  const count = h.events.length
  h.finishAttach()
  await flush()
  assert.equal(dash.calls.includes('load'), false)
  assert.equal(dash.calls.filter((call) => call === 'destroy').length, 1)
  assert.equal(h.events.length, count)
})

test('DASH track selection stays available in the ArtPlayer settings', async () => {
  const h = harness()
  const art = await h.create('dash')
  await flush()
  const dash = h.engines[0]
  const audio = art.settings.get('crayon-audio')
  const subtitle = art.settings.get('crayon-subtitle')
  assert.ok(audio?.selector.length >= 2)
  assert.ok(subtitle?.selector.length >= 2)
  audio.onSelect(audio.selector[1])
  subtitle.onSelect(subtitle.selector[0])
  assert.equal(dash.selectedAudio.language, 'en')
  assert.equal(dash.selectedText, null)
  art.destroy(false)
})

test('adapter failures distinguish network, format and decode without exposing source URLs', async () => {
  for (const [type, detail, expected] of [
    ['NetworkError', 'https://secret.invalid/token', 'network'],
    ['MediaError', 'MediaCodecUnsupported', 'decode'],
    ['MediaError', 'MediaFormatError', 'format'],
  ]) {
    const h = harness()
    const art = await h.create('flv')
    await flush()
    h.engines[0].listeners.get('error')(type, detail)
    const error = h.events.at(-1)
    assert.equal(error.event, 'error')
    assert.equal(error.errorCategory, expected)
    assert.equal(JSON.stringify(error).includes('secret.invalid'), false)
    art.destroy(false)
  }
})

test('HTML and unsupported MSE fail clearly instead of pretending to play', async () => {
  const html = harness()
  const art = await html.create('html')
  await flush()
  assert.equal(html.events.at(-1).errorCategory, 'format')
  assert.equal(art.video.src, '')
  art.destroy(false)
  const flv = harness()
  flv.context.mpegts.getFeatureList = () => ({ mseLivePlayback: false, msePlayback: false })
  const unsupported = await flv.create('flv')
  await flush()
  assert.equal(flv.events.at(-1).errorCategory, 'capability')
  assert.equal(flv.engines.length, 0)
  unsupported.destroy(false)
})
