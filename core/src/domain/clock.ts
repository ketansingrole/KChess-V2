import { rules } from './engine.ts'
import type { Color } from './position.ts'

/** The clock's plain state; every transition is computed by the Rust rules from the time given. */
interface ClockState {
  times: Record<Color, number>
  ticking?: Color
  lastUpdate: number
  emergMs: number
  alerted: Record<Color, boolean>
}

/** A number argument as the rules take it: non-finite values travel as their text. */
const wire = (value: number): number | string => (Number.isFinite(value) ? value : String(value))

/**
 * Client clock modelled on lila's `ui/lib/src/game/clock/clockCtrl.ts`:
 * the server sends authoritative times, the client interpolates from
 * them, optionally delaying the countdown to compensate network lag.
 */
export class Clock {
  /** Created on first use, so the clock needs no rules binding until it is read. */
  private state?: ClockState

  private now: () => number
  constructor(now: () => number = () => performance.now()) {
    this.now = now
  }

  private current(): ClockState {
    this.state ??= rules<ClockState>('clockInitial')
    return this.state
  }

  set(data: {
    white: number
    black: number
    ticking?: Color
    initialSeconds?: number
    delayCentis?: number
  }): void {
    this.state = rules<ClockState>('clockSet', this.current(), data, wire(this.now()))
  }

  pause(): void {
    this.state = rules<ClockState>('clockPause', this.current(), wire(this.now()))
  }

  get running(): Color | undefined {
    return this.current().ticking
  }

  remaining(color: Color, now = this.now()): number {
    return rules<number>('clockRemaining', this.current(), color, wire(now))
  }

  /** True once per player, when that player's clock drops below the low-time threshold. */
  lowTimeAlert(color: Color, now = this.now()): boolean {
    const next = rules<{ state: ClockState; result: boolean }>(
      'clockLowTimeAlert',
      this.current(),
      color,
      wire(now),
    )
    this.state = next.state
    return next.result
  }
}

/** m:ss, h:mm:ss past an hour, and days plus hours for correspondence clocks. */
export function formatClock(millis: number): string {
  return rules<string>('formatClock', wire(millis))
}
