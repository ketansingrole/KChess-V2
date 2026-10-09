import { onMounted, onUnmounted } from 'vue'
import { registerSwipeBackStep, type SwipeBackStep } from '../utils/swipeBack'

/**
 * Step out of the page's inner levels (a broadcast tour, a game, a review)
 * with a two-finger swipe before the swipe leaves the page.
 */
export function useSwipeBack(step: SwipeBackStep): void {
  let unregister: (() => void) | null = null
  onMounted(() => {
    unregister = registerSwipeBackStep(step)
  })
  onUnmounted(() => unregister?.())
}
