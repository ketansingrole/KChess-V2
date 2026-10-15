import { computed, ref } from 'vue'
import { useIntervalFn, useLocalStorage } from '@vueuse/core'
import { formatRunClock } from '@kchess/rules/rush'
import { useCountdown } from '../utils/countdown'

/** Run lengths in seconds for the practice drills; 0 is untimed practice. */
export const RUN_LENGTHS = [
  { label: '30s', value: 30 },
  { label: '1 min', value: 60 },
  { label: '2 min', value: 120 },
  { label: '5 min', value: 300 },
  { label: 'Untimed', value: 0 },
] as const

/**
 * The chosen length of a practice run and its clock: a countdown that ends the run, or for untimed
 * practice a stopwatch that only counts up. Each timed length keeps its own best; the drill's
 * original length keeps the original record name, so earlier bests still count.
 */
export function useRunLength(storageKey: string, defaultSeconds: number, onEnd: () => void) {
  const length = useLocalStorage(storageKey, defaultSeconds)
  const untimed = computed(() => length.value === 0)
  /** Whether the run on screen (or just ended) was untimed, whatever is chosen since. */
  const runUntimed = ref(false)
  const label = computed(
    () => RUN_LENGTHS.find((item) => item.value === length.value)?.label ?? `${length.value}s`,
  )
  const countdown = useCountdown(defaultSeconds * 1000, onEnd)
  let startedAt = 0
  const elapsed = ref(0)
  const stopwatch = useIntervalFn(() => (elapsed.value = performance.now() - startedAt), 250, {
    immediate: false,
  })

  function start(): void {
    runUntimed.value = untimed.value
    if (untimed.value) {
      startedAt = performance.now()
      elapsed.value = 0
      stopwatch.resume()
    } else countdown.start(length.value * 1000)
  }
  function stop(): void {
    countdown.stop()
    stopwatch.pause()
  }
  /** The record name for a run of the chosen length (`find`, `find-120s`, ...). */
  function recordVariant(base: string): string {
    return length.value === defaultSeconds ? base : `${base}-${length.value}s`
  }

  return {
    length,
    untimed,
    runUntimed,
    label,
    start,
    stop,
    recordVariant,
    elapsed,
    /** What the run's clock shows: time left, or time spent when untimed. */
    clock: computed(() => formatRunClock(runUntimed.value ? elapsed.value : countdown.left.value)),
    low: computed(() => !runUntimed.value && countdown.left.value < 10_000),
  }
}
