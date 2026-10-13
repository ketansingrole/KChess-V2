import { ref, watch, onScopeDispose } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import { play } from '../utils/sound'
import { useGameArchive } from './gameArchive'

/**
 * Stand-alone chess clock for a real board, extracted from `stores/local.ts`.
 * `top` is the player across the table. Owns its ticker, expiry, archive and
 * unload flush; the game store only re-exports it so existing callers keep working.
 */
export function useOtbClock(now: { value: number }) {
  const otb = ref<{ top: number; bottom: number }>({ top: 300_000, bottom: 300_000 })
  const otbConfig = ref({ minutes: 5, increment: 3, bottomMinutes: 5 })
  const otbRunning = ref<'top' | 'bottom' | null>(null)
  const otbFlagged = ref<'top' | 'bottom' | null>(null)
  const otbMoves = ref({ top: 0, bottom: 0 })
  let otbStarted = performance.now()
  const clockArchive = useGameArchive(
    () => ({
      source: 'clock',
      setup: STANDARD_SETUP,
      moves: [],
      white: 'Bottom player',
      black: 'Top player',
      result: '*',
      reason: otbFlagged.value
        ? `${otbFlagged.value === 'top' ? 'Top' : 'Bottom'} player ran out of time`
        : 'Clock session',
      timeControl: `${otbConfig.value.bottomMinutes * 60}+${otbConfig.value.increment} / ${otbConfig.value.minutes * 60}+${otbConfig.value.increment}`,
      clockSummary: `Bottom: ${otbMoves.value.bottom} moves · ${Math.ceil(otb.value.bottom / 1000)}s left; Top: ${otbMoves.value.top} moves · ${Math.ceil(otb.value.top / 1000)}s left`,
    }),
    () => otbMoves.value.top + otbMoves.value.bottom > 0 || Boolean(otbFlagged.value),
    false,
  )
  watch(otbFlagged, (flag) => {
    if (flag) clockArchive.save(true)
  })
  // A clock nobody has started yet shows the times as they are typed; a started one keeps its time.
  const otbShownStart = { top: otb.value.top, bottom: otb.value.bottom }
  watch(
    () => ({ top: otbConfig.value.minutes, bottom: otbConfig.value.bottomMinutes }),
    (minutes) => {
      if (otbRunning.value || otbFlagged.value) return
      for (const side of ['top', 'bottom'] as const) {
        const value = Number(minutes[side])
        const untouched = otbMoves.value[side] === 0 && otb.value[side] === otbShownStart[side]
        if (value > 0 && untouched) otb.value[side] = otbShownStart[side] = value * 60_000
      }
    },
  )
  function otbLeft(side: 'top' | 'bottom', at = performance.now()): number {
    void now.value
    if (otbRunning.value !== side) return otb.value[side]
    return Math.max(0, otb.value[side] - (at - otbStarted))
  }
  function expireOtb(at = performance.now()): boolean {
    const side = otbRunning.value
    if (!side || otbLeft(side, at) > 0) return false
    otb.value = { ...otb.value, [side]: 0 }
    otbFlagged.value = side
    otbRunning.value = null
    void play('lowTime')
    return true
  }
  const otbTicker = useIntervalFn(
    () => {
      now.value = performance.now()
      expireOtb(now.value)
    },
    100,
    { immediate: false },
  )
  watch(otbRunning, (side) => (side ? otbTicker.resume() : otbTicker.pause()))
  /** A player pressed their clock: theirs stops (plus increment), the other side's starts. */
  function otbPress(side: 'top' | 'bottom'): void {
    const at = performance.now()
    if (otbFlagged.value || expireOtb(at)) return
    if (otbRunning.value && otbRunning.value !== side) return
    const other = side === 'top' ? 'bottom' : 'top'
    if (otbRunning.value === side) {
      const left = otbLeft(side, at) + otbConfig.value.increment * 1000
      otb.value = { ...otb.value, [side]: left }
      otbMoves.value = { ...otbMoves.value, [side]: otbMoves.value[side] + 1 }
    }
    otbStarted = at
    otbRunning.value = other
  }
  function otbPause(): void {
    const at = performance.now()
    if (expireOtb(at)) return
    const side = otbRunning.value
    if (!side) return
    otb.value = { ...otb.value, [side]: otbLeft(side, at) }
    otbRunning.value = null
  }
  function saveClockSession(): void {
    otbPause()
    clockArchive.save(true)
  }
  window.addEventListener('beforeunload', saveClockSession)
  onScopeDispose(() => {
    saveClockSession()
    window.removeEventListener('beforeunload', saveClockSession)
  })
  function otbReset(): void {
    clockArchive.reset()
    otbRunning.value = null
    otbFlagged.value = null
    otbMoves.value = { top: 0, bottom: 0 }
    otb.value = {
      top: otbConfig.value.minutes * 60_000,
      bottom: otbConfig.value.bottomMinutes * 60_000,
    }
    Object.assign(otbShownStart, otb.value)
  }
  return { otbConfig, otbRunning, otbFlagged, otbMoves, otbLeft, otbPress, otbPause, otbReset }
}
