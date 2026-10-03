import { Chess } from 'chessops/chess'
import { INITIAL_FEN, makeFen, parseFen } from 'chessops/fen'
import { parseSan } from 'chessops/san'
import { makeUci, parseUci } from 'chessops/util'
import type { Judgment, ReviewEval, ReviewSide, ReviewSummary, StoredReview } from './types.ts'

/**
 * Game review, worked out the way Lichess does it (lila's `Advice`, `WinPercent` and
 * `AccuracyPercent`, which are open source): every score becomes a winning chance, and a move is
 * judged by how much of the mover's winning chance it threw away. The formulas are reimplemented
 * here, not copied; the thresholds match Lichess so labels fetched from it and labels worked out
 * locally agree.
 */

type Color = 'white' | 'black'
const other = (color: Color): Color => (color === 'white' ? 'black' : 'white')

/** A short, stable key for a start position and its moves (cyrb53, as hex). */
export function reviewKey(fen: string, moves: readonly string[]): string {
  const text = `${fen}|${moves.join(' ')}`
  let h1 = 0xdeadbeef
  let h2 = 0x41c6ce57
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i)
    h1 = Math.imul(h1 ^ code, 2654435761)
    h2 = Math.imul(h2 ^ code, 1597334677)
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909)
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909)
  return `${(h2 >>> 0).toString(16).padStart(8, '0')}${(h1 >>> 0).toString(16).padStart(8, '0')}`
}

export interface ReplayedPosition {
  fen: string
  turn: Color
  /** The game is over here: no engine is needed for its score. */
  end?: 'checkmate' | 'draw'
}

/**
 * The positions of a game, the start first, as far as its moves are legal. `fen` comes back
 * normalized, so the same game always has the same key.
 */
export function replay(fen: string, moves: readonly string[]): ReplayedPosition[] {
  const setup = parseFen(fen)
  if (setup.isErr) return []
  const created = Chess.fromSetup(setup.value)
  if (created.isErr) return []
  const pos = created.value
  const describe = (): ReplayedPosition => ({
    fen: makeFen(pos.toSetup()),
    turn: pos.turn,
    end: pos.isCheckmate()
      ? 'checkmate'
      : pos.isStalemate() || pos.isInsufficientMaterial()
        ? 'draw'
        : undefined,
  })
  const positions = [describe()]
  for (const uci of moves) {
    const move = parseUci(uci)
    if (!move || !pos.isLegal(move)) break
    pos.play(move)
    positions.push(describe())
  }
  return positions
}

/** A position may carry only the engine's move (Lichess names it without a score). */
export const hasScore = (score: ReviewEval): boolean =>
  score.cp !== undefined || score.mate !== undefined

/** The score of a finished position: checkmate is lost for the side to move, the rest drawn. */
export function endEval(end: 'checkmate' | 'draw'): ReviewEval {
  return end === 'checkmate' ? { mate: 0 } : { cp: 0 }
}

/* ── Winning chances ──────────────────────────────────────────────── */

const MAX_CP = 1000
const clampCp = (cp: number): number => Math.max(-MAX_CP, Math.min(MAX_CP, cp))
const rawChances = (cp: number): number => 2 / (1 + Math.exp(-0.00368208 * cp)) - 1

/**
 * White's winning chances in −1…1. `turn` is the side to move in the position, needed only for
 * a checkmate (`mate: 0`), which is lost for whoever is to move.
 */
export function winChances(score: Pick<ReviewEval, 'cp' | 'mate'>, turn: Color): number {
  if (score.mate === 0) return turn === 'white' ? -1 : 1
  if (score.mate !== undefined) {
    // Lichess counts a shorter mate as a slightly surer win, but every mate as nearly certain.
    const cp = (21 - Math.min(10, Math.abs(score.mate))) * 100
    return rawChances(Math.sign(score.mate) * cp)
  }
  return rawChances(clampCp(score.cp ?? 0))
}

/** A score in centipawns for the average-loss figure: mates count as ±10 pawns. */
function centipawns(score: Pick<ReviewEval, 'cp' | 'mate'>, turn: Color): number {
  if (score.mate === 0) return turn === 'white' ? -MAX_CP : MAX_CP
  if (score.mate !== undefined) return Math.sign(score.mate) * MAX_CP
  return clampCp(score.cp ?? 0)
}

/** From one side's point of view. */
const pov = (color: Color, value: number): number => (color === 'white' ? value : -value)

/* ── Judging a move ───────────────────────────────────────────────── */

const CHANCE_THRESHOLDS: [number, Judgment][] = [
  [0.3, 'blunder'],
  [0.2, 'mistake'],
  [0.1, 'inaccuracy'],
]

/**
 * The label for a move, from the scores before and after it. Like Lichess: walking into a forced
 * mate or letting one slip is judged by how good the position still was, and otherwise by the
 * drop in winning chances.
 */
export function judge(
  before: Pick<ReviewEval, 'cp' | 'mate'>,
  after: Pick<ReviewEval, 'cp' | 'mate'>,
  mover: Color,
): Judgment | undefined {
  const mateBefore = before.mate === undefined ? undefined : pov(mover, before.mate)
  const mateAfter =
    after.mate === undefined || after.mate === 0 ? undefined : pov(mover, after.mate)
  const delivered = after.mate === 0
  const cpBefore = before.cp === undefined ? 0 : pov(mover, clampCp(before.cp))
  const cpAfter = after.cp === undefined ? 0 : pov(mover, clampCp(after.cp))
  if (!delivered) {
    // Walked into a forced mate.
    if (mateBefore === undefined && mateAfter !== undefined && mateAfter < 0)
      return cpBefore < -999 ? 'inaccuracy' : cpBefore < -700 ? 'mistake' : 'blunder'
    // Had a forced mate and let it go.
    if (mateBefore !== undefined && mateBefore > 0 && (mateAfter === undefined || mateAfter < 0))
      return cpAfter > 999 ? 'inaccuracy' : cpAfter > 700 ? 'mistake' : 'blunder'
  }
  if (before.cp === undefined || after.cp === undefined) return undefined
  const drop = pov(mover, rawChances(clampCp(before.cp)) - rawChances(clampCp(after.cp)))
  return CHANCE_THRESHOLDS.find(([threshold]) => drop >= threshold)?.[1]
}

/* ── Accuracy ─────────────────────────────────────────────────────── */

/** A move's accuracy (0–100) from the mover's winning percentage before and after it. */
export function moveAccuracy(winBefore: number, winAfter: number): number {
  if (winAfter >= winBefore) return 100
  const raw = 103.1668100711649 * Math.exp(-0.04354415386753951 * (winBefore - winAfter))
  // Lichess adds one point for the uncertainty of an imperfect analysis.
  return Math.max(0, Math.min(100, raw - 3.166924740191411 + 1))
}

const mean = (values: number[]): number => values.reduce((sum, v) => sum + v, 0) / values.length
function standardDeviation(values: number[]): number {
  const average = mean(values)
  return Math.sqrt(mean(values.map((v) => (v - average) ** 2)))
}
function harmonicMean(values: number[]): number {
  const positive = values.map((v) => Math.max(v, 0.001))
  return positive.length / positive.reduce((sum, v) => sum + 1 / v, 0)
}

/* ── A whole game ─────────────────────────────────────────────────── */

export interface ReviewedMove {
  /** Who played it. */
  color: Color
  judgment?: Judgment
  /** 0–100, when both scores are known. */
  accuracy?: number
  /** White's winning chances (−1…1) after the move, for the chart. */
  chances?: number
}

export interface GameAnalysis {
  moves: ReviewedMove[]
  white: ReviewSide
  black: ReviewSide
  /** White's winning chances in the starting position. */
  startChances?: number
}

/**
 * Labels, accuracy and average loss for every move of a review (as far as it has scores).
 * Lichess's own labels and accuracy are kept when the review came from Lichess.
 */
export function analyseReview(review: StoredReview): GameAnalysis {
  const positions = replay(review.fen, review.moves)
  const evalAt = (i: number): ReviewEval | null => {
    const end = positions[i]?.end
    const stored = review.evals[i]
    if (stored && hasScore(stored)) return stored
    return end ? endEval(end) : null
  }
  const turnAt = (i: number): Color =>
    positions[i]?.turn ?? (i % 2 === 0 ? positions[0]!.turn : other(positions[0]!.turn))
  const percent = (i: number): number | undefined => {
    const score = evalAt(i)
    return score ? 50 + 50 * winChances(score, turnAt(i)) : undefined
  }
  const count = Math.min(review.moves.length, Math.max(0, positions.length - 1))
  const moves: ReviewedMove[] = []
  const losses: Record<Color, number[]> = { white: [], black: [] }
  for (let i = 0; i < count; i++) {
    const color = turnAt(i)
    const before = evalAt(i)
    const after = evalAt(i + 1)
    const move: ReviewedMove = { color }
    if (after) move.chances = winChances(after, turnAt(i + 1))
    const lichess = review.judgments?.[i]
    if (review.source === 'lichess' && review.judgments) move.judgment = lichess ?? undefined
    else if (before && after) move.judgment = judge(before, after, color)
    const winBefore = percent(i)
    const winAfter = percent(i + 1)
    if (winBefore !== undefined && winAfter !== undefined)
      move.accuracy = moveAccuracy(pov(color, winBefore - 50) + 50, pov(color, winAfter - 50) + 50)
    if (before && after)
      losses[color].push(
        Math.max(
          0,
          pov(color, centipawns(before, turnAt(i))) - pov(color, centipawns(after, turnAt(i + 1))),
        ),
      )
    moves.push(move)
  }
  const accuracy = gameAccuracy(moves, count, percent, turnAt)
  const side = (color: Color): ReviewSide => {
    const own = moves.filter((move) => move.color === color)
    const lichess = review.source === 'lichess' ? review.accuracy?.[color] : undefined
    const figure = lichess ?? accuracy[color]
    return {
      ...(figure === undefined ? {} : { accuracy: Math.round(figure) }),
      ...(losses[color].length ? { acpl: Math.round(mean(losses[color])) } : {}),
      inaccuracy: own.filter((move) => move.judgment === 'inaccuracy').length,
      mistake: own.filter((move) => move.judgment === 'mistake').length,
      blunder: own.filter((move) => move.judgment === 'blunder').length,
    }
  }
  const start = evalAt(0)
  return {
    moves,
    white: side('white'),
    black: side('black'),
    ...(start ? { startChances: winChances(start, turnAt(0)) } : {}),
  }
}

/**
 * Lichess's game accuracy: each move's accuracy weighted by how volatile the game was around it,
 * averaged with the plain harmonic mean so one terrible move still shows.
 */
function gameAccuracy(
  moves: ReviewedMove[],
  count: number,
  percent: (i: number) => number | undefined,
  turnAt: (i: number) => Color,
): Partial<Record<Color, number>> {
  const wins: number[] = []
  for (let i = 0; i <= count; i++) {
    const value = percent(i)
    if (value === undefined) return {}
    wins.push(value)
  }
  if (wins.length < 2) return {}
  const size = Math.max(2, Math.min(8, Math.floor(wins.length / 10)))
  const windows: number[][] = []
  for (let i = 0; i < Math.min(size, wins.length) - 2; i++) windows.push(wins.slice(0, size))
  for (let i = 0; i + size <= wins.length; i++) windows.push(wins.slice(i, i + size))
  const weights = windows.map((window) => Math.max(0.5, Math.min(12, standardDeviation(window))))
  const result: Partial<Record<Color, number>> = {}
  for (const color of ['white', 'black'] as const) {
    const scored: { accuracy: number; weight: number }[] = []
    for (let i = 0; i < count; i++) {
      if (turnAt(i) !== color) continue
      const accuracy = moves[i]?.accuracy
      const weight = weights[i]
      if (accuracy !== undefined && weight !== undefined) scored.push({ accuracy, weight })
    }
    if (!scored.length) continue
    const weighted =
      scored.reduce((sum, s) => sum + s.accuracy * s.weight, 0) /
      scored.reduce((sum, s) => sum + s.weight, 0)
    result[color] = (weighted + harmonicMean(scored.map((s) => s.accuracy))) / 2
  }
  return result
}

export function summarize(review: StoredReview): ReviewSummary {
  const { white, black } = analyseReview(review)
  return { key: review.key, source: review.source, complete: review.complete, white, black }
}

/* ── Games as start position + UCI moves ─────────────────────────── */

/** Lichess speeds of standard chess; variants (Chess960, Crazyhouse …) are not reviewed. */
const STANDARD_PERFS = new Set([
  'ultraBullet',
  'bullet',
  'blitz',
  'rapid',
  'classical',
  'correspondence',
])
export const isReviewablePerf = (perf: string): boolean => STANDARD_PERFS.has(perf)

/** SAN moves played from `fen`, as UCI, as far as they are legal. */
export function sanToUci(fen: string, sans: readonly string[]): string[] {
  const setup = parseFen(fen)
  if (setup.isErr) return []
  const created = Chess.fromSetup(setup.value)
  if (created.isErr) return []
  const pos = created.value
  const moves: string[] = []
  for (const san of sans) {
    const move = parseSan(pos, san)
    if (!move) break
    moves.push(makeUci(move))
    pos.play(move)
  }
  return moves
}

/** A synced Lichess game's start position (from its PGN's FEN tag) and UCI moves. */
export function lichessLine(game: { moves: string; pgn?: string | null; initialFen?: string }): {
  fen: string
  moves: string[]
} {
  const tagged = game.pgn ? /\[FEN "([^"]+)"\]/.exec(game.pgn)?.[1] : undefined
  const start = game.initialFen ?? tagged ?? INITIAL_FEN
  const setup = parseFen(start)
  const fen = setup.isErr ? INITIAL_FEN : makeFen(setup.value)
  return { fen, moves: sanToUci(fen, game.moves.split(/\s+/).filter(Boolean)) }
}
