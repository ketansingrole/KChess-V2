/** Rules of the local puzzle runs: Storm and Streak (after Lichess's modes) and Rush (timed, with strikes). */
import { rules } from './engine.ts'
import training from './data/training.json' with { type: 'json' }

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

/** The run kinds (`training.json`, read by `crates/kchess-domain/src/training/rush.rs` too). */
export const RUSH_CONFIGS = training.rushConfigs as Record<string, RushConfig>

/** Configs in the order the picker shows them. */
export const RUSH_CHOICES: string[] = training.rushChoices

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

/** A number as the rules take it: non-finite values travel as their text. */
const num = (value: number): number | string => (Number.isFinite(value) ? value : String(value))

/** Copies a state the rules returned into the one the caller holds. */
function update(state: RushState, next: RushState): void {
  Object.assign(state, next)
}

export function newRush(config: RushConfig): RushState {
  return rules<RushState>('newRush', config)
}

/** Combos that earn time, and how much: 5 → 3 s, 12 → 5 s, 20 → 7 s, 30 → 10 s, then every 10 → 10 s. */
export function comboBonusMs(combo: number): number {
  return rules<number>('comboBonusMs', num(combo))
}

/** Combo progress toward the next bonus, 0–1, for the bar. */
export function comboProgress(combo: number): number {
  return rules<number>('comboProgress', num(combo))
}

/** A correct move. Returns the bonus time it earned. */
export function correctMove(state: RushState, config: RushConfig): number {
  const next = rules<{ state: RushState; result: number }>('correctMove', state, config)
  update(state, next.state)
  return next.result
}

export function puzzleSolved(state: RushState, rating: number): void {
  update(state, rules<RushState>('puzzleSolved', state, num(rating)))
}

/** A wrong move: it breaks the combo and costs time or a strike. */
export function mistake(state: RushState, config: RushConfig): void {
  update(state, rules<RushState>('mistake', state, config))
}

/** Spend a skip; false when none is left. */
export function skip(state: RushState): boolean {
  const next = rules<{ state: RushState; result: boolean }>('skip', state)
  update(state, next.state)
  return next.result
}

export function tick(state: RushState, elapsedMs: number): void {
  update(state, rules<RushState>('tick', state, num(elapsedMs)))
}

export function finish(state: RushState, reason: RushOver): void {
  update(state, rules<RushState>('finish', state, reason))
}

export const accuracy = (state: RushState): number => rules<number>('accuracy', state)

/** `m:ss` (or `m:ss.t` under ten seconds, where tenths matter). */
export function formatRunClock(ms: number): string {
  return rules<string>('formatRunClock', num(ms))
}
