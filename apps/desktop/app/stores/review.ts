import { defineStore } from 'pinia'
import { onScopeDispose, ref, shallowRef, triggerRef } from 'vue'
import type {
  ReviewRequest,
  ReviewStatus,
  ReviewSummary,
  ReviewUpdate,
  StoredReview,
} from '@kchess/contracts/types'

/**
 * Game reviews, as the main process works them out: reviews already seen, the summaries the game
 * list shows, and the review queue's progress. Updates stream in from the main process.
 */
/** Full reviews kept in memory, least recently seen dropped first; the core keeps them all. */
export const REVIEW_CACHE_SIZE = 24

export const useReviewStore = defineStore('review', () => {
  const reviews = shallowRef(new Map<string, StoredReview>())
  /** The review on the board, kept however many others stream in. */
  let pinned = ''
  /** Summaries of reviewed Lichess games, by game id, for the games the list has shown. */
  const summaries = ref<Record<string, ReviewSummary>>({})
  /** When each game's summary last streamed in, so an older lookup cannot overwrite it. */
  const streamedAt = new Map<string, number>()
  /** When each full review last streamed in, by review key, for the same reason. */
  const reviewStreamedAt = new Map<string, number>()
  let tick = 0
  const status = ref<ReviewStatus>({ waiting: 0 })
  let listening = false
  const subscriptions: (() => void)[] = []

  function remember(review: StoredReview): void {
    const cache = reviews.value
    cache.delete(review.key)
    cache.set(review.key, review)
    for (const key of cache.keys()) {
      if (cache.size <= REVIEW_CACHE_SIZE) break
      if (key !== pinned) cache.delete(key)
    }
    triggerRef(reviews)
  }
  function pin(key: string): void {
    pinned = key
  }

  function receive(update: ReviewUpdate): void {
    remember(update.review)
    reviewStreamedAt.set(update.review.key, ++tick)
    const id = update.review.gameId
    if (!id) return
    summaries.value[id] = update.summary
    streamedAt.set(id, tick)
  }

  /**
   * Keep a looked-up review unless a newer one streamed in while the lookup was pending; returns
   * whichever the cache now holds.
   */
  function adopt(review: StoredReview | null, startedAt: number): StoredReview | null {
    if (!review) return null
    if ((reviewStreamedAt.get(review.key) ?? 0) > startedAt) {
      return reviews.value.get(review.key) ?? review
    }
    remember(review)
    return review
  }

  /** Follow the main process's reviews; safe to call more than once. */
  function listen(): void {
    if (listening || typeof window === 'undefined' || !window.kchess) return
    listening = true
    subscriptions.push(window.kchess.onReviewUpdate(receive))
    subscriptions.push(window.kchess.onReviewStatus((next) => (status.value = next)))
    void window.kchess
      .reviewStatus()
      .then((next) => (status.value = next))
      .catch((error: unknown) => {
        console.warn('[review] reading review status failed:', error)
      })
  }

  /**
   * Look up the stored summaries of these games (one history page). Called whenever the page
   * changes, including after a sync, which saves Lichess's own analysis without streaming it.
   */
  async function loadSummaries(ids: readonly string[]): Promise<void> {
    if (!ids.length) return
    const startedAt = tick
    let found: Record<string, ReviewSummary>
    try {
      found = await window.kchess.reviewSummaries([...ids])
    } catch (cause) {
      console.warn('[review] loading review summaries failed:', cause)
      return // The list simply shows no accuracy.
    }
    const next = { ...summaries.value }
    for (const id of ids) {
      if ((streamedAt.get(id) ?? 0) > startedAt) continue
      const summary = found[id]
      if (summary) next[id] = summary
      else delete next[id]
    }
    summaries.value = next
  }

  /** The stored review of these moves, if there is one. */
  async function load(fen: string, moves: string[]): Promise<StoredReview | null> {
    if (!moves.length) return null
    try {
      listen()
      const startedAt = tick
      return adopt(await window.kchess.reviewGet(fen, moves), startedAt)
    } catch (cause) {
      console.warn('[review] loading review failed:', cause)
      return null
    }
  }

  /** Review a game now; returns what is stored so far. */
  async function request(request: ReviewRequest): Promise<StoredReview | null> {
    listen()
    const startedAt = tick
    return adopt(
      await window.kchess.reviewRequest({ ...request, moves: [...request.moves] }),
      startedAt,
    )
  }

  async function cancel(key: string): Promise<void> {
    await window.kchess.reviewCancel(key).catch((error: unknown) => {
      console.warn('[review] cancelling review failed:', error)
    })
  }

  function dispose(): void {
    subscriptions.splice(0).forEach((off) => off())
    listening = false
  }
  onScopeDispose(dispose)

  return {
    dispose,
    reviews,
    pin,
    summaries,
    status,
    listen,
    load,
    request,
    cancel,
    loadSummaries,
  }
})
