import { onMounted, onUnmounted } from 'vue'
import { SubscriptionScope } from '../../../../core/src/domain/requestScope'
import { activeSwipeBackStep, activeSwipeForwardStep } from '../utils/swipeBack'
import { installPageSwipeNavigation } from '../utils/pageSwipe'
import { useKChessStore } from '../stores/kchess'

export function usePageSwipeNavigation(): void {
  const router = useRouter()
  let store: ReturnType<typeof useKChessStore> | null = null
  try {
    store = useKChessStore()
  } catch (cause) {
    console.warn('[swipe-navigation] reading store failed:', cause)
    store = null
  }
  const subscriptions = new SubscriptionScope()
  onMounted(() => {
    subscriptions.attach([
      () =>
        installPageSwipeNavigation(
          document,
          {
            canGo: (direction) => {
              // A page's inner levels (a tour, a game, a review) unwind first;
              // forward redoes the last swipe-back while nothing changed since.
              if (direction < 0 && activeSwipeBackStep()) return true
              if (direction > 0 && activeSwipeForwardStep()) return true
              // Vue Router's history excludes the external page that opened the app.
              return (
                typeof router.options.history.state[direction < 0 ? 'back' : 'forward'] === 'string'
              )
            },
            go: (direction) => {
              if (direction < 0) {
                const step = activeSwipeBackStep()
                if (step) {
                  step.back()
                  return
                }
              }
              if (direction > 0) {
                const step = activeSwipeForwardStep()
                if (step?.forward) {
                  step.forward()
                  return
                }
              }
              router.go(direction)
            },
          },
          {
            // Settings load after first paint; the swipe stays on until they say otherwise.
            isEnabled: () => store?.settings?.swipeNavigation ?? true,
            showFeedback: () => store?.settings?.swipeIndicator ?? true,
          },
        ),
    ])
  })
  onUnmounted(() => subscriptions.detach())
}
