import type { LichessUser, PerfType } from '@kchess/contracts/types'
import { PERF_TYPES } from '@kchess/contracts/types'

export const PERF_LABELS: Record<PerfType, string> = {
  ultraBullet: 'UltraBullet',
  bullet: 'Bullet',
  blitz: 'Blitz',
  rapid: 'Rapid',
  classical: 'Classical',
  correspondence: 'Correspondence',
  chess960: 'Chess960',
  kingOfTheHill: 'King of the Hill',
  threeCheck: 'Three-check',
  antichess: 'Antichess',
  atomic: 'Atomic',
  horde: 'Horde',
  racingKings: 'Racing Kings',
  crazyhouse: 'Crazyhouse',
}

export interface PlayedPerf {
  key: PerfType
  games?: number
  rating?: number
  prov?: boolean
  prog?: number
}

/** Categories the player has games in, most played first. */
export function playedPerfs(user: LichessUser | null | undefined): PlayedPerf[] {
  const all = (user?.perfs ?? {}) as Record<string, Omit<PlayedPerf, 'key'>>
  return PERF_TYPES.filter((key) => (all[key]?.games ?? 0) > 0)
    .flatMap((key) => {
      const entry = all[key]
      return entry ? [{ key, ...entry }] : []
    })
    .sort((a, b) => (b.games ?? 0) - (a.games ?? 0))
}
