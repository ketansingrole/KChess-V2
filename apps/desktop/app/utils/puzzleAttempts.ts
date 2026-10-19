import type {
  Puzzle,
  PuzzleActivityEntry,
  PuzzleAttempt,
  PuzzleAttemptOutcome,
} from '@kchess/contracts/types'
import type { ResultBadge } from './games'

/** A local training attempt's outcome (see `puzzle_progress` in the Rust domain). */
export const ATTEMPT_BADGES = {
  solved: { label: 'Solved', color: 'success', variant: 'soft' },
  hinted: { label: 'With hint', color: 'success', variant: 'outline' },
  partial: { label: 'Partial', color: 'info', variant: 'soft' },
  missed: { label: 'Missed', color: 'error', variant: 'soft' },
  revealed: { label: 'Revealed', color: 'neutral', variant: 'subtle' },
} as const satisfies Record<PuzzleAttemptOutcome, ResultBadge>

/** A result Lichess recorded: a win or a loss, nothing finer. */
export const LICHESS_BADGES = {
  win: ATTEMPT_BADGES.solved,
  loss: ATTEMPT_BADGES.missed,
} as const satisfies Record<'win' | 'loss', ResultBadge>

/** “1/3 moves · 2 wrong”: player moves right at the first try of the solution's, and those wrong. */
export const movesText = (attempt: Pick<PuzzleAttempt, 'found' | 'wrong' | 'total'>): string =>
  `${attempt.found}/${attempt.total} moves${attempt.wrong ? ` · ${attempt.wrong} wrong` : ''}`

/** Correct moves of moves tried, as a whole percentage; undefined before any move. */
export function moveAccuracy(found: number, tried: number): number | undefined {
  return tried > 0 ? Math.round((found / tried) * 100) : undefined
}

/**
 * How an attempt was played: the Train tab's modes (same names and icons), a retry from the
 * history, or an attempt Lichess recorded elsewhere (on lichess.org, another device, or before
 * KChess kept its own log).
 */
export type PuzzleHistoryMode = 'practice' | 'offline' | 'rated' | 'retry' | 'lichess'
export const HISTORY_MODES = {
  practice: { label: 'Practice', icon: 'i-lucide-dumbbell', tip: 'Lichess puzzle, unrated' },
  offline: {
    label: 'Downloaded',
    icon: 'i-lucide-hard-drive',
    tip: 'From the puzzle database on this computer',
  },
  rated: {
    label: 'Rated',
    icon: 'i-lucide-cloud-upload',
    tip: 'Counted in your Lichess puzzle rating',
  },
  retry: { label: 'Retry', icon: 'i-lucide-rotate-ccw', tip: 'Tried again: not counted' },
  lichess: {
    label: 'Lichess',
    icon: 'i-lucide-cloud-download',
    tip: 'Recorded by Lichess: only the result is known',
  },
} as const satisfies Record<PuzzleHistoryMode, { label: string; icon: string; tip: string }>

/** The modes the history can be narrowed to; retries belong to the mode they were played in. */
export const HISTORY_FILTERS = [
  { label: 'All', value: 'all' },
  { label: 'Practice', value: 'practice' },
  { label: 'Downloaded', value: 'offline' },
  { label: 'Rated', value: 'rated' },
  { label: 'Lichess', value: 'lichess' },
] as const
export type PuzzleHistoryFilter = (typeof HISTORY_FILTERS)[number]['value']

/** One row of the puzzle history: an attempt kept here, or one Lichess recorded. */
export interface PuzzleHistoryRow {
  key: string
  at: number
  badge: ResultBadge
  /** “3/5 moves” for local attempts; Lichess records only the result. */
  moves?: string
  puzzle: Puzzle
  source: 'local' | 'lichess'
  mode: PuzzleHistoryMode
  /** The mode it was played in, a retry's included: what the filter matches. */
  playedIn: Exclude<PuzzleHistoryFilter, 'all'>
}

export const rowMatches = (row: PuzzleHistoryRow, filter: PuzzleHistoryFilter): boolean =>
  filter === 'all' || row.playedIn === filter

/** When an attempt was played: the time today and yesterday, the date before that. */
export function playedText(at: number, now: Date = new Date()): string {
  const played = new Date(at)
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  const time = played.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
  if (at >= startOfToday) return time
  if (at >= startOfToday - 86_400_000) return `Yesterday ${time}`
  return played.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    ...(played.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }),
  })
}

/** Lichess's record of an attempt also kept here is the same attempt, played within this window. */
const SAME_ATTEMPT_MS = 15 * 60_000

/** Local attempts and Lichess activity in one list, newest first, each attempt once. */
export function puzzleHistoryRows(
  local: readonly PuzzleAttempt[],
  lichess: readonly PuzzleActivityEntry[],
): PuzzleHistoryRow[] {
  const rows: PuzzleHistoryRow[] = local.map((attempt) => ({
    key: `local-${attempt.id}`,
    at: attempt.at,
    badge: ATTEMPT_BADGES[attempt.outcome],
    moves: movesText(attempt),
    puzzle: attempt.puzzle,
    source: 'local',
    mode: attempt.retry ? 'retry' : attempt.mode,
    playedIn: attempt.mode,
  }))
  for (const entry of lichess) {
    const kept = local.some(
      (attempt) =>
        attempt.puzzle.id === entry.puzzle.id &&
        Math.abs(attempt.at - entry.date) < SAME_ATTEMPT_MS,
    )
    if (kept) continue
    rows.push({
      key: `lichess-${entry.puzzle.id}-${entry.date}`,
      at: entry.date,
      badge: entry.win ? LICHESS_BADGES.win : LICHESS_BADGES.loss,
      puzzle: entry.puzzle,
      source: 'lichess',
      mode: 'lichess',
      playedIn: 'lichess',
    })
  }
  return rows.sort((a, b) => b.at - a.at)
}
