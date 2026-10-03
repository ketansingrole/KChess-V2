import type { Color } from '@lichess-org/chessground/types'

/**
 * Client clock modelled on lila's `ui/lib/src/game/clock/clockCtrl.ts`:
 * the server sends authoritative times, the client interpolates from
 * them, optionally delaying the countdown to compensate network lag.
 */
export class Clock {
  private times: Record<Color, number> = { white: 0, black: 0 }
  private ticking: Color | undefined
  private lastUpdate = 0
  private emergMs = 20_000
  private alerted: Record<Color, boolean> = { white: false, black: false }

  /** lila's low-time threshold: min(60, 12.5% of initial), at least 10s (2s for short games). */
  private static threshold(initial: number): number {
    return (
      1000 * Math.min(60, initial < 60 ? Math.max(2, initial * 0.2) : Math.max(10, initial * 0.125))
    )
  }

  set(data: {
    white: number
    black: number
    ticking?: Color
    initialSeconds?: number
    delayCentis?: number
  }): void {
    this.times = { white: Math.max(0, data.white), black: Math.max(0, data.black) }
    this.ticking = data.ticking
    this.lastUpdate = performance.now() + (data.delayCentis ?? 0) * 10
    if (data.initialSeconds) this.emergMs = Clock.threshold(data.initialSeconds)
    if (data.ticking) this.alerted[data.ticking] = false
  }

  pause(): void {
    this.times = { white: this.remaining('white'), black: this.remaining('black') }
    this.ticking = undefined
  }

  remaining(color: Color, now = performance.now()): number {
    const elapsed = this.ticking === color ? Math.max(0, now - this.lastUpdate) : 0
    return Math.max(0, this.times[color] - elapsed)
  }

  /** True once per player, when that player's clock drops below the low-time threshold. */
  lowTimeAlert(color: Color, now = performance.now()): boolean {
    if (this.alerted[color] || this.remaining(color, now) > this.emergMs) return false
    this.alerted[color] = true
    return true
  }
}

export function formatClock(millis: number): string {
  const seconds = Math.max(0, Math.floor(millis / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}
