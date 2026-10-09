/**
 * Chess variants shared by the renderer (boards, move lists) and main (validation).
 * Rules come from Rust (`position.ts`); Lichess names variants by its own keys.
 */
import { rules } from './engine.ts'
import { INITIAL_FEN, Position } from './position.ts'

/** Lichess variant keys KChess can play. Crazyhouse is missing: its pocket needs a drop UI. */
export const VARIANTS = [
  'standard',
  'chess960',
  'kingOfTheHill',
  'threeCheck',
  'antichess',
  'atomic',
  'horde',
  'racingKings',
] as const
export type Variant = (typeof VARIANTS)[number]
/** Every key Lichess uses, including the ones KChess cannot play. */
export type LichessVariantKey = Variant | 'fromPosition' | 'crazyhouse'

export const VARIANT_LABELS: Record<Variant, string> = {
  standard: 'Standard',
  chess960: 'Chess960',
  kingOfTheHill: 'King of the Hill',
  threeCheck: 'Three-check',
  antichess: 'Antichess',
  atomic: 'Atomic',
  horde: 'Horde',
  racingKings: 'Racing Kings',
}

export const VARIANT_HINTS: Record<Variant, string> = {
  standard: 'Normal chess.',
  chess960: 'Pieces on the back rank are shuffled; castling still works.',
  kingOfTheHill: 'Win by bringing your king to one of the four centre squares.',
  threeCheck: 'Win by giving check three times.',
  antichess: 'Captures are forced; lose all your pieces to win.',
  atomic: 'Captures explode, removing every piece next to them except pawns.',
  horde: 'White has 36 pawns and must checkmate; Black must capture them all.',
  racingKings: 'No checks allowed; the first king to reach the eighth rank wins.',
}

/** A game's starting point: the rules and the position before the first move. */
export interface GameSetup {
  variant: Variant
  /** FEN before the first move; for standard games, the usual start. */
  fen: string
}

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

/**
 * The Chess960 starting position numbered `n` (0–959) in the Scharnagl scheme; 518 is the
 * standard start. Castling rights use X-FEN letters so every engine and Lichess can read it.
 */
export function chess960Fen(n: number): string {
  const index = ((Math.floor(n) % 960) + 960) % 960
  const rank: (string | undefined)[] = Array(8).fill(undefined)
  let rest = index
  const light = rest % 4
  rest = Math.floor(rest / 4)
  rank[light * 2 + 1] = 'b'
  const dark = rest % 4
  rest = Math.floor(rest / 4)
  rank[dark * 2] = 'b'
  const queen = rest % 6
  rest = Math.floor(rest / 6)
  const free = (): number[] => rank.flatMap((piece, square) => (piece ? [] : [square]))
  rank[free()[queen]!] = 'q'
  const knights = [
    [0, 1],
    [0, 2],
    [0, 3],
    [0, 4],
    [1, 2],
    [1, 3],
    [1, 4],
    [2, 3],
    [2, 4],
    [3, 4],
  ][rest]!
  const open = free()
  rank[open[knights[0]!]!] = 'n'
  rank[open[knights[1]!]!] = 'n'
  const [a, b, c] = free()
  rank[a!] = 'r'
  rank[b!] = 'k'
  rank[c!] = 'r'
  const black = rank.join('')
  return `${black}/pppppppp/8/8/8/8/PPPPPPPP/${black.toUpperCase()} w KQkq - 0 1`
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

/**
 * Replay UCI moves under a setup's rules. Castling is accepted as king-to-rook or king-two-squares
 * (Lichess sends either); the result keeps the moves as given. Stops at the first illegal move.
 */
export function replaySetup(
  setup: GameSetup,
  moves: readonly string[],
): { position: Position; played: ReplayedMove[]; start: Position } | undefined {
  const start = setupStart(setup)
  const replayed =
    start && rules<{ played: ReplayedMove[]; fen: string } | null>('setupReplay', setup, moves)
  if (!start || !replayed) return undefined
  const position = replayed.played.length
    ? Position.from({ variant: setup.variant, fen: replayed.fen })!
    : start
  return { position, played: replayed.played, start }
}

/** True when the setup is the ordinary standard game (so Stockfish, the explorer and reviews apply). */
export function isStandardStart(setup: GameSetup): boolean {
  return setup.variant === 'standard' && setup.fen === INITIAL_FEN
}

/** Engines can only play standard rules (and Chess960 with UCI_Chess960). */
export function engineSupports(variant: Variant): boolean {
  return variant === 'standard' || variant === 'chess960'
}
