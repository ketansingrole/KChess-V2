/**
 * Chess variants shared by the renderer (boards, move lists) and main (validation).
 * Rules come from Rust (`position.ts`); Lichess names variants by its own keys.
 */
import { rules } from './engine.ts'
import variantText from '../../kchess-domain/data/variants.json' with { type: 'json' }
import { INITIAL_FEN, Position } from './position.ts'

import { VARIANTS } from '@kchess/contracts/variant'
import type { GameSetup, Variant } from '@kchess/contracts/variant'
export { VARIANTS }
export type { GameSetup, Variant } from '@kchess/contracts/variant'

/** Every key Lichess uses, including the ones KChess cannot play. */
export type LichessVariantKey = Variant | 'fromPosition' | 'crazyhouse'

const VARIANT_TEXT = variantText as readonly { key: Variant; label: string; hint: string }[]

/** Each variant's name (shared data, `data/variants.json`). */
export const VARIANT_LABELS = Object.fromEntries(
  VARIANT_TEXT.map(({ key, label }) => [key, label]),
) as Record<Variant, string>

/** What sets each variant apart, in a line. */
export const VARIANT_HINTS = Object.fromEntries(
  VARIANT_TEXT.map(({ key, hint }) => [key, hint]),
) as Record<Variant, string>

export const STANDARD_SETUP: GameSetup = { variant: 'standard', fen: INITIAL_FEN }

export function isVariant(value: unknown): value is Variant {
  return typeof value === 'string' && (VARIANTS as readonly string[]).includes(value)
}

/** The playable variant for a Lichess key; `fromPosition` is standard rules from another start. */
export function variantFromLichess(key: string | undefined): Variant | undefined {
  if (!key || key === 'fromPosition') return 'standard'
  return isVariant(key) ? key : undefined
}

/** Chess960 needs king-takes-rook castling in UCI and on the board. */
export const isChess960 = (variant: Variant): boolean => variant === 'chess960'

/** The position a setup describes, or undefined when its FEN is not legal under its rules. */
export function setupStart(setup: GameSetup): Position | undefined {
  return Position.from(setup)
}

/** Lichess's standard start for each variant (Chess960 has no single one). */
export function defaultFen(variant: Variant): string {
  if (variant === 'chess960') return chess960Fen(518)
  return rules<string>('defaultFen', variant)
}

/** A Chess960 position number as typed: a whole number from 0 to 959. */
export const valid960 = (n: unknown): n is number =>
  typeof n === 'number' && Number.isInteger(n) && n >= 0 && n <= 959

/**
 * The Chess960 starting position numbered `n` (0–959) in the Scharnagl scheme; 518 is the
 * standard start. Castling rights use X-FEN letters so every engine and Lichess can read it.
 */
export function chess960Fen(n: number): string {
  return rules<string>('chess960Fen', n)
}

/** A random Chess960 start. */
export function randomChess960(): string {
  return chess960Fen(Math.floor(Math.random() * 960))
}

/** One replayed move. */
export interface ReplayedMove {
  uci: string
  san: string
}

type Replayed = { position: Position; played: ReplayedMove[]; start: Position } | undefined
/** The last replay of each move list (kept weak, so a replaced list is collected). */
const replayCache = new WeakMap<readonly string[], { key: string; result: Replayed }>()

/**
 * Replay UCI moves under a setup's rules. Castling is accepted as king-to-rook or king-two-squares
 * (Lichess sends either); the result keeps the moves as given. Stops at the first illegal move.
 */
export function replaySetup(setup: GameSetup, moves: readonly string[]): Replayed {
  // A board asks for the same replay for its position and its move list. The answer is shared
  // only for the same list with the same text: a new list (a restart, even at the same position)
  // still gets a new position, which watchers read as a change, and a list edited in place is
  // replayed again. Reading every move keeps a reactive caller's dependencies complete.
  const key = `${setup.variant}|${setup.fen}|${moves.join(' ')}`
  const cached = replayCache.get(moves)
  if (cached?.key === key) return cached.result

  const start = setupStart(setup)
  const replayed =
    start && rules<{ played: ReplayedMove[]; fen: string } | null>('setupReplay', setup, moves)
  let result: Replayed
  if (start && replayed) {
    if (!replayed.played.length) result = { position: start, played: replayed.played, start }
    else {
      const position = Position.from({ variant: setup.variant, fen: replayed.fen })
      if (position) result = { position, played: replayed.played, start }
    }
  }
  replayCache.set(moves, { key, result })
  return result
}

/** True when the setup is the ordinary standard game (so Stockfish, the explorer and reviews apply). */
export function isStandardStart(setup: GameSetup): boolean {
  return setup.variant === 'standard' && setup.fen === INITIAL_FEN
}

/** Engines can only play standard rules (and Chess960 with UCI_Chess960). */
export function engineSupports(variant: Variant): boolean {
  return variant === 'standard' || variant === 'chess960'
}
