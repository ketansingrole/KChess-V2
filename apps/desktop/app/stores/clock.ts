import { ref, shallowRef, watch, onScopeDispose } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import {
  otbClockArchive,
  otbClockConfigure,
  otbClockExpire,
  otbClockLeft,
  otbClockNew,
  otbClockPause,
  otbClockPress,
  otbClockReset,
  otbClockStart,
  type OtbClockState,
  type OtbConfig,
  type OtbSide,
} from '@kchess/rules/otbClock'
import { play } from '../utils/sound'
import { useGameArchive } from './gameArchive'

/** A typed time as the rules take it: a cleared field counts as no time. */
const minutes = (value: unknown): number => (Number.isFinite(Number(value)) ? Number(value) : 0)
const configOf = (config: OtbConfig): OtbConfig => ({
  minutes: minutes(config.minutes),
  bottomMinutes: minutes(config.bottomMinutes),
  increment: minutes(config.increment),
})

/**
 * Stand-alone chess clock for a real board, extracted from `stores/local.ts`.
 * `top` is the player across the table. The Rust rules (`games/otb_clock.rs`, which the GPUI app
 * uses too) make every decision; this keeps their state, owns its ticker, expiry, archive and
 * unload flush. The game store only re-exports it so existing callers keep working.
 */
export function useOtbClock(now: { value: number }) {
  const otbConfig = ref<OtbConfig>({ minutes: 5, increment: 3, bottomMinutes: 5 })
  const clock = shallowRef<OtbClockState>(otbClockNew(configOf(otbConfig.value)))
  const otbRunning = ref<OtbSide | null>(null)
  const otbFlagged = ref<OtbSide | null>(null)
  const otbMoves = ref({ top: 0, bottom: 0 })
  /** Keep the rules' answer and show it. */
  function keep(next: OtbClockState): void {
    clock.value = next
    otbRunning.value = next.running
    otbFlagged.value = next.flagged
    otbMoves.value = { ...next.moves }
  }
  const clockArchive = useGameArchive(
    () => {
      const archive = otbClockArchive(clock.value)
      return {
        source: 'clock',
        setup: STANDARD_SETUP,
        moves: [],
        white: 'Bottom player',
        black: 'Top player',
        result: '*',
        reason: archive.reason,
        timeControl: archive.timeControl,
        clockSummary: archive.clockSummary,
      }
    },
    () => otbClockArchive(clock.value).hasPlay,
    false,
  )
  watch(otbFlagged, (flag) => {
    if (flag) clockArchive.save(true)
  })
  // A clock nobody has started yet shows the times as they are typed; a started one keeps its time.
  watch(otbConfig, (config) => keep(otbClockConfigure(clock.value, configOf(config))), {
    deep: true,
  })
  function otbLeft(side: OtbSide, at = performance.now()): number {
    void now.value
    return otbClockLeft(clock.value, side, at)
  }
  /** A transition's state, with the low-time sound when a flag fell. */
  function moved(result: { clock: OtbClockState; flagFell: boolean }): boolean {
    keep(result.clock)
    if (result.flagFell) void play('lowTime')
    return result.flagFell
  }
  function expireOtb(at = performance.now()): boolean {
    return moved(otbClockExpire(clock.value, at))
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
  function otbPress(side: OtbSide): void {
    moved(otbClockPress(clock.value, side, performance.now()))
  }
  function otbPause(): void {
    moved(otbClockPause(clock.value, performance.now()))
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
    keep(otbClockReset(clock.value))
  }
  /** Start (or resume) the clock; after a flag, Start begins a fresh session at the set times. */
  function otbStart(): void {
    // A fresh session after a flag is a new Game history entry.
    if (otbFlagged.value) clockArchive.reset()
    moved(otbClockStart(clock.value, performance.now()))
  }
  return {
    otbConfig,
    otbRunning,
    otbFlagged,
    otbMoves,
    otbLeft,
    otbPress,
    otbPause,
    otbReset,
    otbStart,
  }
}
