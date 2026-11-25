import type Artplayer from 'artplayer'
import type { HistoryItem } from '@shared/types'
import { formatClock } from './format'

type HistoryInput = Omit<HistoryItem, 'updatedAt'>
type PlaybackEntry = Omit<HistoryInput, 'position' | 'duration'>

export function trackPlaybackHistory(
  art: Artplayer,
  entry: PlaybackEntry,
  previous: HistoryItem | undefined,
  record: (input: HistoryInput) => Promise<void>
): () => void {
  const resumeAt =
    previous &&
    previous.siteKey === entry.siteKey &&
    previous.vodId === entry.vodId &&
    previous.lineIndex === entry.lineIndex &&
    previous.epIndex === entry.epIndex &&
    Number.isFinite(previous.position) &&
    Number.isFinite(previous.duration) &&
    previous.position > 0 &&
    previous.position < previous.duration
      ? previous.position
      : 0
  let resumePending = resumeAt > 0
  let stopped = false
  let lastSaved = 0

  const restore = (): void => {
    if (stopped || !resumePending || !Number.isFinite(art.duration) || art.duration <= 0) return
    resumePending = false
    // HLS 的 metadata 可能先于有效时长到达；重复事件也不能把用户拖回旧位置。
    if (resumeAt < art.duration) {
      art.currentTime = resumeAt
      art.notice.show = `已继续播放至 ${formatClock(resumeAt)}`
    }
  }

  const save = (force = false): void => {
    if (stopped) return
    restore()
    const now = Date.now()
    if (!force && now - lastSaved < 5000) return
    lastSaved = now
    const duration = Number.isFinite(art.duration) && art.duration > 0 ? Math.floor(art.duration) : 0
    const position = Number.isFinite(art.currentTime) ? Math.max(0, Math.floor(art.currentTime)) : 0
    void record({ ...entry, position, duration }).catch((err: Error) => {
      if (!stopped) art.notice.show = `观看进度保存失败：${err.message}`
    })
  }
  const onTimeUpdate = (): void => save()
  const onSave = (): void => save(true)
  art.on('video:loadedmetadata', restore)
  art.on('video:durationchange', restore)
  art.on('video:canplay', restore)
  art.on('video:timeupdate', onTimeUpdate)
  art.on('video:pause', onSave)
  art.on('video:ended', onSave)
  save(true)

  return () => {
    if (stopped) return
    // 先读出当前时间，再销毁播放器，否则最后不足 5 秒的进度会丢失。
    save(true)
    stopped = true
    art.off('video:loadedmetadata', restore)
    art.off('video:durationchange', restore)
    art.off('video:canplay', restore)
    art.off('video:timeupdate', onTimeUpdate)
    art.off('video:pause', onSave)
    art.off('video:ended', onSave)
  }
}
