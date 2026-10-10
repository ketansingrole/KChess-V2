/** Lichess Board API time-control rules, shared by UI and Electron main (`records/time_control.rs`). */
import { rules } from './engine.ts'

export type LichessPerf = 'UltraBullet' | 'Bullet' | 'Blitz' | 'Rapid' | 'Classical'

/** Lichess perf formula: estimated game length = initial time + 40 × increment. */
export function totalSeconds(minutes: number, increment: number): number {
  return rules<number>('totalSeconds', minutes, increment)
}

export function perfFor(minutes: number, increment: number): LichessPerf {
  return rules<LichessPerf>('perfFor', minutes, increment)
}

/** Public matchmaking (`POST /api/board/seek`): Rapid and slower only. */
export function canBoardSeek(minutes: number, increment: number): boolean {
  return rules<boolean>('canBoardSeek', minutes, increment)
}

/** Direct challenge (`POST /api/challenge/{username}`): Blitz and slower. */
export function canDirectChallenge(minutes: number, increment: number): boolean {
  return rules<boolean>('canDirectChallenge', minutes, increment)
}

export function canPlayOnline(minutes: number, increment: number, targeted: boolean): boolean {
  return rules<boolean>('canPlayOnline', minutes, increment, targeted)
}
