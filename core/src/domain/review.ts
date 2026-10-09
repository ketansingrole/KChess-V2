import type { Judgment, ReviewEval, ReviewSide, StoredReview } from '../contracts/types.ts'
import { rules } from './engine.ts'

/**
 * Game review, worked out the way Lichess does it (lila's `Advice`, `WinPercent` and
 * `AccuracyPercent`, which are open source): every score becomes a winning chance, and a move is
 * judged by how much of the mover's winning chance it threw away. The formulas are reimplemented
 * (in `crates/kchess-domain/src/review.rs`), not copied; the thresholds match Lichess so labels
 * fetched from it and labels worked out locally agree.
 */

type Color = 'white' | 'black'

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
  return rules<ReplayedPosition[]>('replay', fen, moves)
}

/** A position may carry only the engine's move (Lichess names it without a score). */
export const hasScore = (score: ReviewEval): boolean =>
  score.cp !== undefined || score.mate !== undefined

/** The score of a finished position: checkmate is lost for the side to move, the rest drawn. */
export function endEval(end: 'checkmate' | 'draw'): ReviewEval {
  return end === 'checkmate' ? { mate: 0 } : { cp: 0 }
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
  const analysis = rules<GameAnalysis | null>('analyseReview', review)
  // The rules decline only a review whose start position cannot be replayed.
  if (!analysis) throw new Error('This review cannot be analysed.')
  return analysis
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
