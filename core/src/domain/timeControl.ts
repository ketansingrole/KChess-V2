/** Lichess Board API time-control rules, shared by UI and Electron main. */

export type LichessPerf = 'UltraBullet' | 'Bullet' | 'Blitz' | 'Rapid' | 'Classical'

/** Lichess perf formula: estimated game length = initial time + 40 × increment. */
export function totalSeconds(minutes: number, increment: number): number {
  return minutes * 60 + 40 * increment
}

export function perfFor(minutes: number, increment: number): LichessPerf {
  const total = totalSeconds(minutes, increment)
  if (total < 30) return 'UltraBullet'
  if (total < 180) return 'Bullet'
  if (total < 480) return 'Blitz'
  if (total < 1500) return 'Rapid'
  return 'Classical'
}

/** Public matchmaking (`POST /api/board/seek`): Rapid and slower only. */
export function canBoardSeek(minutes: number, increment: number): boolean {
  return totalSeconds(minutes, increment) >= 480
}

/** Direct challenge (`POST /api/challenge/{username}`): Blitz and slower. */
export function canDirectChallenge(minutes: number, increment: number): boolean {
  return totalSeconds(minutes, increment) >= 180
}

export function canPlayOnline(minutes: number, increment: number, targeted: boolean): boolean {
  return targeted ? canDirectChallenge(minutes, increment) : canBoardSeek(minutes, increment)
}
