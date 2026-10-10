import { nativeCallSync } from './nativeCore'
import type { ReviewSummary, StoredReview } from '../contracts/types'

/**
 * Game reviews on disk (`crates/kchess-core/src/store/reviews.rs`). A review is the scores of every
 * position; the summary beside it (accuracy and counts per side) is what the game list shows
 * without working anything out.
 */

export function readReview(key: string): StoredReview | null {
  return nativeCallSync<StoredReview | null>('store.reviewStore.readReview', key)
}

/** Save `review` unless a better one is already stored; returns what is stored afterwards. */
export function writeReview(review: StoredReview): {
  review: StoredReview
  summary: ReviewSummary
} {
  return nativeCallSync<{ review: StoredReview; summary: ReviewSummary }>(
    'store.reviewStore.writeReview',
    review,
  )
}

/**
 * Summaries of these Lichess games' reviews, by game id; games without one are left out.
 * When a game has several, the finished, most recent one wins.
 */
export function reviewSummaries(ids: readonly string[]): Record<string, ReviewSummary> {
  return nativeCallSync<Record<string, ReviewSummary>>('store.reviewStore.reviewSummaries', ids)
}

/** Whether this account is still on this device (it may have been logged out or removed). */
export function hasAccount(username: string): boolean {
  return nativeCallSync<boolean>('store.reviewStore.hasAccount', username)
}

/** Remember that Lichess was asked for these games' analysis. */
export function markChecked(ids: readonly string[], at = Date.now()): void {
  nativeCallSync('store.reviewStore.markChecked', ids, at)
}

export interface GameToReview {
  id: string
  account: string
  moves: string
  pgn: string | null
  perf: string
  createdAt: number
  /** Lichess has already been asked for its analysis. */
  checked: boolean
}

/**
 * The accounts' finished games with no finished review, newest first. `since` limits them to games played
 * after it (ms).
 */
export function gamesToReview(accounts: readonly string[], since = 0, limit = 300): GameToReview[] {
  return nativeCallSync<GameToReview[]>('store.reviewStore.gamesToReview', accounts, since, limit)
}

/** Reviews kept, for the storage report. */
export function reviewCount(): number {
  return nativeCallSync<number>('store.reviewStore.reviewCount')
}
