import type { StoredReview } from '../contracts/types'
import { rules } from './engine.ts'

/** Every claim comes from a scored, legally replayed position; no generated chess facts. */
export function explainReviewedMove(review: StoredReview, index: number): string {
  return rules<string>('explainReviewedMove', review, index)
}
