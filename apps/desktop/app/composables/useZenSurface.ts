import { onBeforeUnmount, toValue, watchEffect, type MaybeRefOrGetter } from 'vue'
import { useKChessStore } from '../stores/kchess'

/**
 * Declares that this page shows a game board with its zen mode toggle while `shown` holds. Zen mode
 * applies only then, so the toggle that turns it on is always on screen to turn it off again.
 */
export function useZenSurface(shown: MaybeRefOrGetter<boolean>): void {
  const store = useKChessStore()
  const token = Symbol('zen surface')
  watchEffect(() => store.setZenSurface(token, toValue(shown)))
  onBeforeUnmount(() => store.setZenSurface(token, false))
}
