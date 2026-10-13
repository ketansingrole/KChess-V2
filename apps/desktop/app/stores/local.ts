import { defineStore } from 'pinia'
import { computed, reactive, ref, toRefs, watch } from 'vue'
import { useIntervalFn, useLocalStorage } from '@vueuse/core'
import { INITIAL_FEN } from '@kchess/rules/position'
import { initialLibrary, persistSession } from '../utils/library'
import type { LocalClock } from '@kchess/rules/library'
import {
  checkColor,
  lastMoveKeys,
  setupDests,
  setupPositionAfter,
  setupSanHistory,
} from '@kchess/rules/chess'
import { type GameSetup } from '@kchess/rules/variant'
import { play, playMoveSound } from '../utils/sound'

import { useGameArchive } from './gameArchive'
import { useOtbClock } from './clock'

import { LocalGame, localState, type Color } from '@kchess/rules/gameSession'
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

  /* ── Stand-alone chess clock (extracted; API unchanged) ──────────── */
  const { otbConfig, otbRunning, otbFlagged, otbMoves, otbLeft, otbPress, otbPause, otbReset } =
    useOtbClock(now)

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
