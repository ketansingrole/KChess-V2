import { defineStore } from 'pinia'
import { onScopeDispose, ref, shallowRef, triggerRef } from 'vue'
import type {
  ReviewRequest,
  ReviewStatus,
  ReviewSummary,
  ReviewUpdate,
  StoredReview,
} from '../../src/shared/types'

/**
 * Game reviews, as the main process works them out: reviews already seen, the summaries the game
 * list shows, and the review queue's progress. Updates stream in from the main process.
 */
export const useReviewStore = defineStore('review', () => {
  const reviews = shallowRef(new Map<string, StoredReview>())
  /** Summaries of reviewed Lichess games, by game id. */
  const summaries = ref<Record<string, ReviewSummary>>({})
  const status = ref<ReviewStatus>({ waiting: 0 })
  let listening = false
  const subscriptions: (() => void)[] = []

  function remember(review: StoredReview): void {
    reviews.value.set(review.key, review)
    triggerRef(reviews)
  }

  function receive(update: ReviewUpdate): void {
    remember(update.review)
    if (update.review.gameId) summaries.value[update.review.gameId] = update.summary
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
      .catch(() => undefined)
    void loadSummaries()
  }

  async function loadSummaries(): Promise<void> {
    try {
      summaries.value = await window.kchess.reviewSummaries()
    } catch {
      // The list simply shows no accuracy.
    }
  }

  /** The stored review of these moves, if there is one. */
  async function load(fen: string, moves: string[]): Promise<StoredReview | null> {
    if (!moves.length) return null
    try {
      listen()
      const review = await window.kchess.reviewGet(fen, moves)
      if (review) remember(review)
      return review
    } catch {
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
    await window.kchess.reviewCancel(key).catch(() => undefined)
  }

  function dispose(): void {
    subscriptions.splice(0).forEach((off) => off())
    listening = false
  }
  onScopeDispose(dispose)

  return { dispose, reviews, summaries, status, listen, load, request, cancel, loadSummaries }
})
