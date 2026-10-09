import { defineStore } from 'pinia'
import { computed, reactive, ref, toRefs, watch, onScopeDispose } from 'vue'
import { useIntervalFn, useLocalStorage } from '@vueuse/core'
import { INITIAL_FEN } from 'chessops/fen'
import { initialLibrary, persistSession } from '../utils/library'
import type { LocalClock } from '../../../../core/src/domain/library'
import {
  checkColor,
  lastMoveKeys,
  setupDests,
  setupPositionAfter,
  setupSanHistory,
} from '../../../../core/src/domain/chess'
import { STANDARD_SETUP, type GameSetup } from '../../../../core/src/domain/variant'
import { play, playMoveSound } from '../utils/sound'

import { useGameArchive } from './gameArchive'

import { LocalGame, localState, type Color } from '../../../../core/src/domain/gameSession'
export type { LocalClock }

/** Two people at one computer, and a stand-alone chess clock for a real board. */
export const useLocalGameStore = defineStore('local', () => {
  const saved = initialLibrary().sessions.local
  const state = reactive(localState(saved))
  const { setup, moves, clock, times, paused } = toRefs(state)
  const game = new LocalGame(
    state,
    () => performance.now(),
    () => {
      void play('lowTime')
    },
  )
  const ply = ref<number | null>(null)
  /** Turn the board towards whoever is to move. */
  const autoFlipOn = useLocalStorage('kchess:local-autoflip', false)
  const autoFlip = computed({
    get: () => autoFlipOn.value,
    // Switching the automatic turning off leaves the board facing the way it is now.
    set: (on: boolean) => {
      if (!on && autoFlipOn.value && !over.value) fixedOrientation.value = position.value.turn
      autoFlipOn.value = on
    },
  })
  const fixedOrientation = ref<Color>('white')
  const saveError = persistSession('local', () => game.snapshot())

  const position = computed(() => game.position)
  const shownPly = computed(() => Math.min(ply.value ?? Infinity, moves.value.length))
  const display = computed(() =>
    shownPly.value === moves.value.length
      ? position.value
      : setupPositionAfter(setup.value, moves.value.slice(0, shownPly.value)),
  )
  const history = computed(() => setupSanHistory(setup.value, moves.value))
  const lastMove = computed(() => lastMoveKeys(moves.value.slice(0, shownPly.value)))
  const dests = computed(() => setupDests(setup.value, position.value))
  const check = computed(() => checkColor(display.value))
  const result = computed(() => game.result)
  const over = computed(() => game.over)
  const atLive = computed(() => shownPly.value === moves.value.length)
  const orientation = computed<Color>(() =>
    autoFlip.value && !over.value ? position.value.turn : fixedOrientation.value,
  )

  const archive = useGameArchive(
    () => game.archiveSnapshot(),
    () => moves.value.length > 0 || Boolean(result.value),
  )

  /* ── Clock ───────────────────────────────────────────────────────── */
  const running = computed(() => game.running)
  const now = ref(performance.now())
  const ticker = useIntervalFn(
    () => {
      now.value = performance.now()
      game.expire(now.value)
    },
    100,
    { immediate: false },
  )
  watch(
    () => Boolean(times.value) && !over.value,
    (on) => (on ? ticker.resume() : ticker.pause()),
    { immediate: true },
  )
  function clockText(color: Color): string | undefined {
    if (!times.value) return undefined
    void now.value
    const ms = game.remaining(color)
    if (ms < 10_000) return `0:${(ms / 1000).toFixed(1).padStart(4, '0')}`
    const seconds = Math.ceil(ms / 1000)
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
  }

  function move(uci: string): boolean {
    if (!atLive.value) return false
    const san = game.move(uci)
    if (!san) return false
    ply.value = null
    playMoveSound(san)
    return true
  }
  function takeback(): void {
    game.takeback()
    ply.value = null
  }
  function start(next: GameSetup = setup.value, nextClock: LocalClock | null = clock.value): void {
    archive.reset()
    game.start(next, nextClock)
    ply.value = null
    fixedOrientation.value = 'white'
  }
  function resign(color: Color): void {
    game.resign(color)
  }
  function agreeDraw(): void {
    game.agreeDraw()
  }
  function togglePause(): void {
    game.togglePause()
  }
  /** Flipping by hand always turns the board, so it also stops the automatic turning. */
  function flip(): void {
    const shown = orientation.value
    autoFlipOn.value = false
    fixedOrientation.value = shown === 'white' ? 'black' : 'white'
  }
  function view(next: number): void {
    const clamped = Math.max(0, Math.min(next, moves.value.length))
    ply.value = clamped === moves.value.length ? null : clamped
  }

  /* ── Stand-alone chess clock ─────────────────────────────────────── */
  /** Times for a real board: `top` is the player across the table. */
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

  return {
    setup,
    moves,
    shownPly,
    clock,
    times,
    paused,
    autoFlip,
    saveError,
    position,
    display,
    history,
    lastMove,
    dests,
    check,
    result,
    over,
    atLive,
    orientation,
    running,
    clockText,
    move,
    takeback,
    start,
    resign,
    agreeDraw,
    togglePause,
    flip,
    view,
    otbConfig,
    otbRunning,
    otbFlagged,
    otbMoves,
    otbLeft,
    otbPress,
    otbPause,
    otbReset,
    standardStart: INITIAL_FEN,
  }
})
