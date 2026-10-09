import type { StoredReview } from '../contracts/types'
import { replay } from './review'
import { formatEval, pvSan } from './analysisTree'
/** Every claim comes from a scored, legally replayed position; no generated chess facts. */
export function explainReviewedMove(review: StoredReview, index: number): string {
  if (index < 0 || index >= review.moves.length) return ''
  const positions = replay(review.fen, review.moves)
  const before = review.evals[index],
    after = review.evals[index + 1]
  const fen = positions[index]?.fen
  if (!before || !after || !fen) return ''
  const played = pvSan(fen, [review.moves[index]!], 1)[0]
  if (!played) return ''
  const suggested = pvSan(
    fen,
    before.pv?.[0] === before.best ? (before.pv ?? []) : before.best ? [before.best] : [],
    6,
  )
  const source =
    review.source === 'lichess'
      ? 'Lichess analysis'
      : `Stockfish${before.depth ? ` at depth ${before.depth}` : ''}`
  const score = (value: typeof before): string =>
    value.cp === undefined && value.mate === undefined ? 'unknown' : formatEval(value)
  return `${source}: after ${played.san}, the evaluation changed from ${score(before)} to ${score(after)} (White's perspective).${suggested.length ? ` Preferred line: ${suggested.map((move) => move.label).join(' ')}.` : ''}${review.complete ? '' : ' This review is still provisional.'}`
}
