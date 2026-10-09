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
  return rules<string>('reviewKey', fen, moves)
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
export const hasScore = (score: ReviewEval): boolean => rules<boolean>('hasScore', score)

/** The score of a finished position: checkmate is lost for the side to move, the rest drawn. */
export function endEval(end: 'checkmate' | 'draw'): ReviewEval {
  return rules<ReviewEval>('endEval', end)
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
export const isReviewablePerf = (perf: string): boolean => rules<boolean>('isReviewablePerf', perf)
