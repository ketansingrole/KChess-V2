import { defineStore } from 'pinia'
import { computed, ref, watch, onScopeDispose } from 'vue'
import { useIntervalFn, useLocalStorage } from '@vueuse/core'
import { INITIAL_FEN } from 'chessops/fen'
import { initialLibrary, persistSession } from '../utils/library'
import type { LocalClock } from '../../src/shared/library'
import { boardResult, pgnResult, timeoutWinner } from '../../src/shared/gameResult'
import {
  checkColor,
  lastMoveKeys,
  setupDests,
  setupDrawReason,
  setupPositionAfter,
  setupSanHistory,
} from '../../src/shared/chess'
import { replaySetup, STANDARD_SETUP, type GameSetup } from '../../src/shared/variant'
import { play, playMoveSound } from '../utils/sound'

import { useGameArchive } from './gameArchive'

type Color = 'white' | 'black'
export type { LocalClock }

/** Two people at one computer, and a stand-alone chess clock for a real board. */
export const useLocalGameStore = defineStore('local', () => {
  const saved = initialLibrary().sessions.local
  const setup = ref<GameSetup>(saved?.setup ?? STANDARD_SETUP)
  const moves = ref<string[]>(saved?.moves ?? [])
  const ply = ref<number | null>(null)
  const clock = ref<LocalClock | null>(saved?.clock ?? null)
  const times = ref<{ white: number; black: number } | null>(saved?.times ?? null)
  /** A result decided off the board: resignation, agreement or the clock. */
  const declared = ref<{ winner?: Color; reason: string } | null>(saved?.result ?? null)
  const paused = ref(true)
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
  let turnStarted = performance.now()
  const saveError = persistSession('local', () => ({
    setup: setup.value,
    moves: moves.value,
    clock: clock.value,
    times: times.value ? { white: remaining('white'), black: remaining('black') } : null,
    result: declared.value,
  }))

  const position = computed(() => setupPositionAfter(setup.value, moves.value))
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
  const draw = computed(() => setupDrawReason(setup.value, moves.value))
  const result = computed<{ winner?: Color; reason: string } | null>(() => {
    if (declared.value) return declared.value
    return boardResult(position.value, setup.value.variant, draw.value)
  })
  const over = computed(() => Boolean(result.value))
  const atLive = computed(() => shownPly.value === moves.value.length)
  const orientation = computed<Color>(() =>
    autoFlip.value && !over.value ? position.value.turn : fixedOrientation.value,
  )

  const archive = useGameArchive(
    () => ({
      source: 'board',
      setup: setup.value,
      moves: [...moves.value],
      white: 'White',
      black: 'Black',
      result: pgnResult(Boolean(result.value), result.value?.winner),
      reason: result.value?.reason ?? 'In progress',
      timeControl: clock.value
        ? `${clock.value.white.minutes * 60}+${clock.value.white.increment} / ${clock.value.black.minutes * 60}+${clock.value.black.increment}`
        : '-',
    }),
    () => moves.value.length > 0 || Boolean(result.value),
  )

  /* ── Clock ───────────────────────────────────────────────────────── */
  const running = computed(
    () => Boolean(times.value) && !over.value && !paused.value && moves.value.length > 0,
  )
  function remaining(color: Color, now = performance.now()): number {
    const left = times.value?.[color] ?? 0
    if (!running.value || position.value.turn !== color) return left
    return Math.max(0, left - (now - turnStarted))
  }
  function expireClock(at = performance.now()): boolean {
    if (!running.value) return false
    const turn = position.value.turn
    if (remaining(turn, at) > 0) return false
    times.value = { ...times.value!, [turn]: 0 }
    const winner = timeoutWinner(position.value, turn)
    declared.value = winner
      ? { winner, reason: 'Time out' }
      : { reason: 'Time out, but mate was impossible' }
    void play('lowTime')
    return true
  }
  const now = ref(performance.now())
  const ticker = useIntervalFn(
    () => {
      now.value = performance.now()
      expireClock(now.value)
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
    const ms = remaining(color)
    if (ms < 10_000) return `0:${(ms / 1000).toFixed(1).padStart(4, '0')}`
    const seconds = Math.ceil(ms / 1000)
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
  }

  function move(uci: string): boolean {
    if (over.value || !atLive.value) return false
    const nowMs = performance.now()
    if (expireClock(nowMs)) return false
    const replayed = replaySetup(setup.value, [...moves.value, uci])
    if (replayed?.played.length !== moves.value.length + 1) return false
    const mover = position.value.turn
    if (times.value) {
      const spent = running.value ? nowMs - turnStarted : 0
      const increment = (clock.value?.[mover].increment ?? 0) * 1000
      times.value = {
        ...times.value,
        [mover]: Math.max(0, times.value[mover] - spent) + (moves.value.length ? increment : 0),
      }
      turnStarted = nowMs
    }
    // The first move starts the clock.
    paused.value = false
    moves.value = [...moves.value, uci]
    ply.value = null
    playMoveSound(replayed.played.at(-1)?.san)
    return true
  }
  function takeback(): void {
    if (!moves.value.length) return
    moves.value = moves.value.slice(0, -1)
    declared.value = null
    ply.value = null
    turnStarted = performance.now()
  }
  function start(next: GameSetup = setup.value, nextClock: LocalClock | null = clock.value): void {
    archive.reset()
    setup.value = next
    clock.value = nextClock
    moves.value = []
    ply.value = null
    declared.value = null
    paused.value = true
    times.value = nextClock
      ? { white: nextClock.white.minutes * 60_000, black: nextClock.black.minutes * 60_000 }
      : null
    turnStarted = performance.now()
    fixedOrientation.value = 'white'
  }
  function resign(color: Color): void {
    if (over.value) return
    declared.value = {
      winner: color === 'white' ? 'black' : 'white',
      reason: `${color === 'white' ? 'White' : 'Black'} resigned`,
    }
  }
  function agreeDraw(): void {
    if (!over.value) declared.value = { reason: 'Draw agreed' }
  }
  function togglePause(): void {
    const at = performance.now()
    if (!times.value || over.value || expireClock(at)) return
    if (!paused.value) {
      // Freeze the side to move's time where it is.
      const turn = position.value.turn
      times.value = { ...times.value, [turn]: remaining(turn, at) }
    }
    paused.value = !paused.value
    turnStarted = at
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
