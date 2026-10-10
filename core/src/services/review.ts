import { nativeCall, nativeCallSync, onNativeEvent } from './nativeCore.ts'
import { scopedState } from './platform.ts'
import { logWarn } from './logger.ts'
import type { ReviewRequest, ReviewStatus, ReviewUpdate, StoredReview } from '../contracts/types'

/**
 * Game review, run by the Rust core (`crates/kchess-core/src/engine/review.rs`): the queue, the
 * review engine and the automatic reviews of synced games. The core's settings, accounts and busy
 * state come from the store and the engines; Lichess analysis is fetched through the host
 * (`lichess.reviews`, transitional). Updates and status arrive as `review:update` and
 * `review:status` events.
 */

/** Where review updates and status go (the frontend's event emitter). */
export interface ReviewSink {
  update: (update: ReviewUpdate) => void
  status: (status: ReviewStatus) => void
}

const forwarding = scopedState(() => ({
  unsubscribe: [] as (() => void)[],
  sink: undefined as ReviewSink | undefined,
}))

/** Starts the review queue and forwards its events to `sink`. Automatic reviews look for work soon. */
export function setupReviews(sink: ReviewSink): void {
  forwarding.sink = sink
  for (const unsubscribe of forwarding.unsubscribe) unsubscribe()
  forwarding.unsubscribe = [
    onNativeEvent<ReviewUpdate>('review:update', (update) => forwarding.sink?.update(update)),
    onNativeEvent<ReviewStatus>('review:status', (status) => forwarding.sink?.status(status)),
  ]
  void nativeCall('reviews.setup').catch((cause: unknown) => {
    logWarn('review', 'Review setup failed:', cause)
  })
}

export const reviewStatus = (): ReviewStatus => nativeCallSync<ReviewStatus>('reviews.status')

/** The stored review of these moves, if any (`fen` and moves as the caller has them). */
export const getReview = (fen: string, moves: string[]): StoredReview | null =>
  nativeCallSync<StoredReview | null>('reviews.get', fen, moves)

/**
 * Review a game now, ahead of automatic reviews. Resolves with what is already stored (complete or
 * not); updates arrive as the review goes on.
 */
export function requestReview(request: ReviewRequest): Promise<StoredReview | null> {
  return nativeCall<StoredReview | null>('reviews.request', request)
}

/** Stop reviewing these moves (what is done so far is kept). */
export function cancelReview(key: string): void {
  nativeCallSync('reviews.cancel', key)
}

/** Logout must prevent an interrupted review from restoring deleted account data. */
export function discardAccountReviews(accounts: string[]): void {
  nativeCallSync('reviews.discardAccounts', accounts)
}

/** Settings changed or a sync finished: look again for work. */
export function reviewsChanged(): void {
  void nativeCall('reviews.changed').catch((cause: unknown) => {
    logWarn('review', 'Review wake-up failed:', cause)
  })
}

/** The engine was deleted or replaced: drop it (the review in progress stops, keeping its work). */
export function restartReviewEngine(): void {
  nativeCallSync('reviews.restartEngine')
}

/** The app is quitting. Resolves once no review is running. */
export async function stopReviews(): Promise<void> {
  await nativeCall('reviews.stop')
}
