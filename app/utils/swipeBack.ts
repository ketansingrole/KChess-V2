/** One inner level a page can step out of before a swipe leaves the page. */
export interface SwipeBackStep {
  /** True while the page has an inner level to step out of. */
  canBack: () => boolean
  /** Step out one inner level. */
  back: () => void
  /** True while the last swipe-back can be redone (nothing changed since). */
  canForward?: () => boolean
  /** Re-enter the level the last swipe-back left. */
  forward?: () => void
}

let active: SwipeBackStep | null = null

/**
 * Offer the swipe the page's inner levels first (a tour, a game, a review).
 * Unmounting only clears its own registration, so overlapping mounts during
 * navigation cannot drop the incoming page's steps.
 */
export function registerSwipeBackStep(step: SwipeBackStep): () => void {
  active = step
  let done = false
  return () => {
    if (done) return
    done = true
    if (active === step) active = null
  }
}

/** The offered step that can go back, if any. */
export function activeSwipeBackStep(): SwipeBackStep | null {
  return active?.canBack() ? active : null
}

/** The offered step whose swipe-back can be redone, if any. */
export function activeSwipeForwardStep(): SwipeBackStep | null {
  return active?.canForward?.() ? active : null
}
