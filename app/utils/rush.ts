/** Rules of the local puzzle runs: Storm and Streak (after Lichess's modes) and Rush (timed, with strikes). */

export const RUSH_MODES = ['storm', 'streak', 'rush'] as const
export type RushMode = (typeof RUSH_MODES)[number]

export interface RushConfig {
  mode: RushMode
  /** The kind of run, saved with the score (`3min`, `5min`, `survival`; `standard` for the others). */
  variant: string
  title: string
  /** One line of the rules. */
  rules: string
  durationMs?: number
  /** Mistakes that end the run; unset means mistakes cost time instead. */
  strikes?: number
  skips: number
  /** Time lost per mistake. */
  penaltyMs: number
  /** Correct moves in a row earn bonus time (Storm). */
  combo: boolean
  /** Puzzles go from `from` to `to` in rating over `count` steps. */
  ladder: { from: number; to: number; count: number }
}

export const RUSH_CONFIGS: Record<string, RushConfig> = {
  storm: {
    mode: 'storm',
    variant: 'standard',
    title: 'Storm',
    rules:
      'Three minutes. Solve as many puzzles as you can. A run of correct moves earns bonus time; a mistake costs 10 seconds and breaks the run.',
    durationMs: 180_000,
    skips: 0,
    penaltyMs: 10_000,
    combo: true,
    ladder: { from: 800, to: 2600, count: 100 },
  },
  streak: {
    mode: 'streak',
    variant: 'standard',
    title: 'Streak',
    rules:
      'No clock. Every puzzle is harder than the last and the first mistake ends the run. You may skip one puzzle.',
    strikes: 1,
    skips: 1,
    penaltyMs: 0,
    combo: false,
    ladder: { from: 600, to: 2700, count: 100 },
  },
  '3min': {
    mode: 'rush',
    variant: '3min',
    title: 'Rush · 3 minutes',
    rules: 'Three minutes and three strikes. A wrong move is a strike and moves you on.',
    durationMs: 180_000,
    strikes: 3,
    skips: 0,
    penaltyMs: 0,
    combo: false,
    ladder: { from: 600, to: 2600, count: 100 },
  },
  '5min': {
    mode: 'rush',
    variant: '5min',
    title: 'Rush · 5 minutes',
    rules: 'Five minutes and three strikes. A wrong move is a strike and moves you on.',
    durationMs: 300_000,
    strikes: 3,
    skips: 0,
    penaltyMs: 0,
    combo: false,
    ladder: { from: 600, to: 2700, count: 150 },
  },
  survival: {
    mode: 'rush',
    variant: 'survival',
    title: 'Rush · Survival',
    rules: 'No clock, three strikes. How far up the difficulty ladder can you get?',
    strikes: 3,
    skips: 0,
    penaltyMs: 0,
    combo: false,
    ladder: { from: 600, to: 2800, count: 150 },
  },
}

/** Configs in the order the picker shows them. */
export const RUSH_CHOICES: string[] = ['storm', 'streak', '3min', '5min', 'survival']

export type RushOver = 'time' | 'strikes' | 'complete' | 'ended'

export interface RushState {
  /** Puzzles solved. */
  score: number
  /** Correct moves played. */
  moves: number
  mistakes: number
  combo: number
  maxCombo: number
  /** Time left; unset when the run has no clock. */
  timeLeftMs?: number
  skipsLeft: number
  /** Rating of the hardest puzzle solved. */
  highest: number
  /** Bonus time earned in total. */
  bonusMs: number
  over?: RushOver
}

export function newRush(config: RushConfig): RushState {
  return {
    score: 0,
    moves: 0,
    mistakes: 0,
    combo: 0,
    maxCombo: 0,
    timeLeftMs: config.durationMs,
    skipsLeft: config.skips,
    highest: 0,
    bonusMs: 0,
  }
}

/** Combos that earn time, and how much: 5 → 3 s, 12 → 5 s, 20 → 7 s, 30 → 10 s, then every 10 → 10 s. */
const COMBO_LEVELS: readonly [number, number][] = [
  [5, 3000],
  [12, 5000],
  [20, 7000],
  [30, 10_000],
]
export function comboBonusMs(combo: number): number {
  const level = COMBO_LEVELS.find(([at]) => at === combo)
  if (level) return level[1]
  return combo > 30 && combo % 10 === 0 ? 10_000 : 0
}

/** Combo progress toward the next bonus, 0–1, for the bar. */
export function comboProgress(combo: number): number {
  const previous = [...COMBO_LEVELS.map(([at]) => at)].reverse().find((at) => at <= combo)
  const next =
    COMBO_LEVELS.map(([at]) => at).find((at) => at > combo) ??
    (combo < 30 ? 30 : Math.floor(combo / 10) * 10 + 10)
  const base = combo >= 30 ? Math.floor(combo / 10) * 10 : (previous ?? 0)
  return Math.min(1, (combo - base) / (next - base))
}

/** A correct move. Returns the bonus time it earned. */
export function correctMove(state: RushState, config: RushConfig): number {
  if (state.over) return 0
  state.moves++
  state.combo++
  state.maxCombo = Math.max(state.maxCombo, state.combo)
  const bonus = config.combo ? comboBonusMs(state.combo) : 0
  if (bonus && state.timeLeftMs !== undefined) {
    state.timeLeftMs += bonus
    state.bonusMs += bonus
  }
  return bonus
}

export function puzzleSolved(state: RushState, rating: number): void {
  if (state.over) return
  state.score++
  state.highest = Math.max(state.highest, rating)
}

/** A wrong move: it breaks the combo and costs time or a strike. */
export function mistake(state: RushState, config: RushConfig): void {
  if (state.over) return
  state.mistakes++
  state.combo = 0
  if (config.penaltyMs && state.timeLeftMs !== undefined) {
    state.timeLeftMs -= config.penaltyMs
    if (state.timeLeftMs <= 0) finish(state, 'time')
  }
  if (config.strikes && state.mistakes >= config.strikes) finish(state, 'strikes')
}

/** Spend a skip; false when none is left. */
export function skip(state: RushState): boolean {
  if (state.over || state.skipsLeft <= 0) return false
  state.skipsLeft--
  return true
}

export function tick(state: RushState, elapsedMs: number): void {
  if (state.over || state.timeLeftMs === undefined) return
  state.timeLeftMs -= elapsedMs
  if (state.timeLeftMs <= 0) finish(state, 'time')
}

export function finish(state: RushState, reason: RushOver): void {
  if (state.over) return
  state.over = reason
  if (state.timeLeftMs !== undefined) state.timeLeftMs = Math.max(0, state.timeLeftMs)
}

export const accuracy = (state: RushState): number => {
  const total = state.moves + state.mistakes
  return total ? Math.round((state.moves / total) * 100) : 0
}

/** `m:ss` (or `m:ss.t` under ten seconds, where tenths matter). */
export function formatRunClock(ms: number): string {
  const clamped = Math.max(0, ms)
  const totalSeconds = Math.floor(clamped / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = String(totalSeconds % 60).padStart(2, '0')
  return clamped < 10_000 && clamped > 0
    ? `0:${seconds}.${Math.floor((clamped % 1000) / 100)}`
    : `${minutes}:${seconds}`
}
