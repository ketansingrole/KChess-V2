import { defineStore } from 'pinia'
import { onScopeDispose, ref, shallowRef, triggerRef } from 'vue'
import type {
  ReviewRequest,
  ReviewStatus,
  ReviewSummary,
  ReviewUpdate,
  StoredReview,
} from '../../../../core/src/contracts/types'

/**
 * Game reviews, as the main process works them out: reviews already seen, the summaries the game
 * list shows, and the review queue's progress. Updates stream in from the main process.
 */
export const useReviewStore = defineStore('review', () => {
  const reviews = shallowRef(new Map<string, StoredReview>())
  /** Summaries of reviewed Lichess games, by game id, for the games the list has shown. */
  const summaries = ref<Record<string, ReviewSummary>>({})
  /** When each game's summary last streamed in, so an older lookup cannot overwrite it. */
  const streamedAt = new Map<string, number>()
  let tick = 0
  const status = ref<ReviewStatus>({ waiting: 0 })
  let listening = false
  const subscriptions: (() => void)[] = []

  function remember(review: StoredReview): void {
    reviews.value.set(review.key, review)
    triggerRef(reviews)
  }

  function receive(update: ReviewUpdate): void {
    remember(update.review)
    const id = update.review.gameId
    if (!id) return
    summaries.value[id] = update.summary
    streamedAt.set(id, ++tick)
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
      const review = await window.kchess.reviewGet(fen, moves)
      if (review) remember(review)
      return review
    } catch (cause) {
      console.warn('[review] loading review failed:', cause)
      return null
    }
  }

  /** Review a game now; returns what is stored so far. */
  async function request(request: ReviewRequest): Promise<StoredReview | null> {
    listen()
    const review = await window.kchess.reviewRequest({ ...request, moves: [...request.moves] })
    if (review) remember(review)
    return review
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

  return { dispose, reviews, summaries, status, listen, load, request, cancel, loadSummaries }
})
