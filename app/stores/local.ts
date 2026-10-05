import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import { useIntervalFn, useLocalStorage } from '@vueuse/core'
import { INITIAL_FEN } from 'chessops/fen'
import { readSession, persistSession } from '../utils/sessionPersistence'
import {
  checkColor,
  lastMoveKeys,
  setupDests,
  setupDrawReason,
  setupPositionAfter,
  setupSanHistory,
} from '../utils/chess'
import { UCI_MOVE } from '../../src/shared/patterns'
import {
  isVariant,
  replaySetup,
  STANDARD_SETUP,
  type GameSetup,
  type Variant,
} from '../../src/shared/variant'
import { play, playMoveSound } from '../utils/sound'

type Color = 'white' | 'black'
/** Each side's clock: minutes and seconds added after every move (sides may differ, for odds). */
export interface LocalClock {
  white: { minutes: number; increment: number }
  black: { minutes: number; increment: number }
}

const VARIANT_WIN: Partial<Record<Variant, string>> = {
  kingOfTheHill: 'King reached the centre',
  threeCheck: 'Third check',
  antichess: 'Lost every piece',
  atomic: 'King exploded',
  horde: 'Horde captured',
  racingKings: 'King reached the eighth rank',
}

/** Two people at one computer, and a stand-alone chess clock for a real board. */
export const useLocalGameStore = defineStore('local', () => {
  const saved = readSession('kchess:local:v1', (raw) => {
    const value = raw as {
      version?: unknown
      setup?: { variant?: unknown; fen?: unknown }
      moves?: unknown
      clock?: LocalClock | null
      times?: { white?: unknown; black?: unknown } | null
      result?: { winner?: unknown; reason?: unknown } | null
    }
    if (!value || value.version !== 1 || !value.setup || !isVariant(value.setup.variant)) return
    if (typeof value.setup.fen !== 'string' || value.setup.fen.length > 120) return
    const setup = { variant: value.setup.variant, fen: value.setup.fen }
    if (
      !Array.isArray(value.moves) ||
      value.moves.length > 1024 ||
      !value.moves.every((m) => typeof m === 'string' && UCI_MOVE.test(m)) ||
      replaySetup(setup, value.moves)?.played.length !== value.moves.length
    )
      return
    const ms = (n: unknown): number =>
      typeof n === 'number' && Number.isFinite(n) ? Math.max(0, n) : 0
    return {
      setup,
      moves: value.moves as string[],
      clock: value.clock ?? null,
      times: value.times ? { white: ms(value.times.white), black: ms(value.times.black) } : null,
      result:
        value.result && typeof value.result.reason === 'string'
          ? {
              winner:
                value.result.winner === 'white' || value.result.winner === 'black'
                  ? (value.result.winner as Color)
                  : undefined,
              reason: value.result.reason.slice(0, 80),
            }
          : null,
    }
  })
  const setup = ref<GameSetup>(saved?.setup ?? STANDARD_SETUP)
  const moves = ref<string[]>(saved?.moves ?? [])
  const ply = ref<number | null>(null)
  const clock = ref<LocalClock | null>(saved?.clock ?? null)
  const times = ref<{ white: number; black: number } | null>(saved?.times ?? null)
  /** A result decided off the board: resignation, agreement or the clock. */
  const declared = ref<{ winner?: Color; reason: string } | null>(saved?.result ?? null)
  const paused = ref(true)
  /** Turn the board towards whoever is to move. */
  const autoFlip = useLocalStorage('kchess:local-autoflip', false)
  const fixedOrientation = ref<Color>('white')
  let turnStarted = performance.now()
  const saveError = persistSession('kchess:local:v1', () => ({
    version: 1,
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
    const pos = position.value
    const outcome = pos.outcome()
    if (outcome) {
      const reason = pos.isCheckmate()
        ? 'Checkmate'
        : pos.isStalemate()
          ? 'Stalemate'
          : pos.isVariantEnd()
            ? (VARIANT_WIN[setup.value.variant] ?? 'Game over')
            : 'Insufficient material'
      return { winner: outcome.winner, reason }
    }
    if (draw.value) return { reason: draw.value }
    return null
  })
  const over = computed(() => Boolean(result.value))
  const atLive = computed(() => shownPly.value === moves.value.length)
  const orientation = computed<Color>(() =>
    autoFlip.value && !over.value ? position.value.turn : fixedOrientation.value,
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
  const now = ref(performance.now())
  const ticker = useIntervalFn(
    () => {
      now.value = performance.now()
      if (!running.value) return
      const turn = position.value.turn
      if (remaining(turn) <= 0) {
        times.value = { ...times.value!, [turn]: 0 }
        const winner = turn === 'white' ? 'black' : 'white'
        declared.value = position.value.hasInsufficientMaterial(winner)
          ? { reason: 'Time out, but mate was impossible' }
          : { winner, reason: 'Time out' }
        void play('lowTime')
      }
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
    const replayed = replaySetup(setup.value, [...moves.value, uci])
    if (replayed?.played.length !== moves.value.length + 1) return false
    const mover = position.value.turn
    if (times.value) {
      const nowMs = performance.now()
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
    if (!times.value || over.value) return
    if (!paused.value) {
      // Freeze the side to move's time where it is.
      const turn = position.value.turn
      times.value = { ...times.value, [turn]: remaining(turn) }
    }
    paused.value = !paused.value
    turnStarted = performance.now()
  }
  function flip(): void {
    fixedOrientation.value = fixedOrientation.value === 'white' ? 'black' : 'white'
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
  function otbLeft(side: 'top' | 'bottom'): number {
    void now.value
    if (otbRunning.value !== side) return otb.value[side]
    return Math.max(0, otb.value[side] - (performance.now() - otbStarted))
  }
  const otbTicker = useIntervalFn(
    () => {
      now.value = performance.now()
      const side = otbRunning.value
      if (side && otbLeft(side) <= 0) {
        otb.value = { ...otb.value, [side]: 0 }
        otbFlagged.value = side
        otbRunning.value = null
        void play('lowTime')
      }
    },
    100,
    { immediate: false },
  )
  watch(otbRunning, (side) => (side ? otbTicker.resume() : otbTicker.pause()))
  /** A player pressed their clock: theirs stops (plus increment), the other side's starts. */
  function otbPress(side: 'top' | 'bottom'): void {
    if (otbFlagged.value) return
    if (otbRunning.value && otbRunning.value !== side) return
    const other = side === 'top' ? 'bottom' : 'top'
    if (otbRunning.value === side) {
      const left = otbLeft(side) + otbConfig.value.increment * 1000
      otb.value = { ...otb.value, [side]: left }
      otbMoves.value = { ...otbMoves.value, [side]: otbMoves.value[side] + 1 }
    }
    otbStarted = performance.now()
    otbRunning.value = other
  }
  function otbPause(): void {
    const side = otbRunning.value
    if (!side) return
    otb.value = { ...otb.value, [side]: otbLeft(side) }
    otbRunning.value = null
  }
  function otbReset(): void {
    otbRunning.value = null
    otbFlagged.value = null
    otbMoves.value = { top: 0, bottom: 0 }
    otb.value = {
      top: otbConfig.value.minutes * 60_000,
      bottom: otbConfig.value.bottomMinutes * 60_000,
    }
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
