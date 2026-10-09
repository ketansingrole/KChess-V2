import { onBeforeUnmount, ref } from 'vue'
import { useIntervalFn } from '@vueuse/core'

/** A countdown that follows the wall clock (so a slow frame never adds time) and calls `onEnd` at zero. */
export function useCountdown(totalMs: number, onEnd: () => void) {
  const left = ref(totalMs)
  const running = ref(false)
  let last = 0
  const interval = useIntervalFn(
    () => {
      const now = performance.now()
      left.value = Math.max(0, left.value - (now - last))
      last = now
      if (left.value <= 0) {
        stop()
        onEnd()
      }
    },
    100,
    { immediate: false },
  )
  function start(ms = totalMs): void {
    left.value = ms
    last = performance.now()
    running.value = true
    interval.resume()
  }
  function stop(): void {
    running.value = false
    interval.pause()
  }
  onBeforeUnmount(stop)
  return { left, running, start, stop }
}
