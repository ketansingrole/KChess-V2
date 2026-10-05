import { computed, ref, watch, type Ref } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import type { EngineLevel, NotificationKind } from '../../../src/shared/types'
import {
  checkColor,
  lastMoveKeys,
  playUci,
  setupDests,
  setupDrawReason,
  setupPositionAfter,
  setupSanHistory,
  statusText,
  turnColor,
} from '../../utils/chess'
import { readSession, persistSession } from '../../utils/sessionPersistence'
import { useGameArchive } from '../gameArchive'
import { ENGINE_LEVELS } from '../../../src/shared/types'
import { UCI_MOVE } from '../../../src/shared/patterns'
import { engineLevelInfo } from '../../../src/shared/engineLevels'
import {
  engineSupports,
  isStandardStart,
  isVariant,
  replaySetup,
  STANDARD_SETUP,
  type GameSetup,
} from '../../../src/shared/variant'
import { playMoveSound, play } from '../../utils/sound'

/** A clock for computer games: minutes each, plus seconds added after every move. */
export interface ComputerClock {
  minutes: number
  increment: number
}

/** Computer-game state lives for the store's lifetime, including across route changes. */
export function useComputerGame(options: {
  engineReady: Ref<boolean>
  assistanceAllowed?: () => boolean
  fail: (cause: unknown) => void
  recheckEngine: () => Promise<void>
  notifyDesktop: (kind: NotificationKind, title: string, body: string) => void
}) {
  const { engineReady, fail, recheckEngine, notifyDesktop } = options
  const allowed = () => options.assistanceAllowed?.() ?? true
  const saved = readSession('kchess:computer:v1', (raw) => {
    if (!raw || typeof raw !== 'object') return undefined
    const value = raw as {
      version?: unknown
      moves?: unknown
      ply?: unknown
      level?: unknown
      color?: unknown
      resigned?: unknown
      setup?: { variant?: unknown; fen?: unknown }
      clock?: { minutes?: unknown; increment?: unknown } | null
      times?: { white?: unknown; black?: unknown }
      flagged?: unknown
    }
    // Version 1 games began at the standard start and had no clock.
    const setup: GameSetup =
      value.version === 2 &&
      value.setup &&
      isVariant(value.setup.variant) &&
      typeof value.setup.fen === 'string' &&
      value.setup.fen.length <= 100
        ? { variant: value.setup.variant, fen: value.setup.fen }
        : STANDARD_SETUP
    if (
      (value.version !== 1 && value.version !== 2) ||
      !engineSupports(setup.variant) ||
      !Array.isArray(value.moves) ||
      value.moves.length > 1024 ||
      !value.moves.every((m) => typeof m === 'string' && UCI_MOVE.test(m)) ||
      replaySetup(setup, value.moves)?.played.length !== value.moves.length ||
      !ENGINE_LEVELS.includes(value.level as EngineLevel) ||
      !['white', 'black'].includes(String(value.color))
    )
      return undefined
    const clock =
      value.clock &&
      Number.isInteger(value.clock.minutes) &&
      Number.isInteger(value.clock.increment) &&
      (value.clock.minutes as number) >= 0 &&
      (value.clock.minutes as number) <= 180 &&
      (value.clock.increment as number) >= 0 &&
      (value.clock.increment as number) <= 180
        ? { minutes: value.clock.minutes as number, increment: value.clock.increment as number }
        : null
    const time = (n: unknown): number =>
      typeof n === 'number' && Number.isFinite(n) ? Math.max(0, n) : 0
    return {
      moves: value.moves as string[],
      ply:
        typeof value.ply === 'number'
          ? Math.max(0, Math.min(value.moves.length, Math.floor(value.ply)))
          : value.moves.length,
      level: value.level as EngineLevel,
      color: value.color as 'white' | 'black',
      resigned: value.resigned === true,
      setup,
      clock,
      times: clock ? { white: time(value.times?.white), black: time(value.times?.black) } : null,
      flagged:
        value.flagged === 'white' || value.flagged === 'black'
          ? (value.flagged as 'white' | 'black')
          : null,
    }
  })
  const localMoves = ref<string[]>(saved?.moves ?? [])
  const localPly = ref(saved?.ply ?? 0)
  const level = ref<EngineLevel>(saved?.level ?? 'club')
  const userColor = ref<'white' | 'black'>(saved?.color ?? 'white')
  /** Rules and start of the game: standard, Chess960 or a set-up position. */
  const localSetup = ref<GameSetup>(saved?.setup ?? STANDARD_SETUP)
  /** The clock chosen for new games (null: no clock). */
  const localClock = ref<ComputerClock | null>(saved?.clock ?? null)
  /** Milliseconds left for each side in the current game, when it has a clock. */
  const times = ref<{ white: number; black: number } | null>(saved?.times ?? null)
  /** The side whose time ran out. */
  const flagged = ref<'white' | 'black' | null>(saved?.flagged ?? null)
  /** performance.now() when the side to move started thinking. */
  let turnStarted = performance.now()
  const thinking = ref(false)
  const gameEpoch = ref(0)
  /** The player gave up; the game is over whatever the position. */
  const resigned = ref(saved?.resigned ?? false)
  watch([engineReady, allowed], ([ready, permitted], [, wasPermitted]) => {
    const now = performance.now()
    if (
      !permitted &&
      wasPermitted &&
      times.value &&
      !localOver.value &&
      localMoves.value.length >= 2
    ) {
      const turn = localGame.value.turn
      times.value = { ...times.value, [turn]: Math.max(0, times.value[turn] - (now - turnStarted)) }
    }
    if (permitted !== wasPermitted) turnStarted = now
    if (ready && permitted && saved) void computerTurn()
  })
  const localGame = computed(() => setupPositionAfter(localSetup.value, localMoves.value))
  const localDraw = computed(() => setupDrawReason(localSetup.value, localMoves.value))
  const localOver = computed(
    () =>
      resigned.value ||
      Boolean(flagged.value) ||
      localGame.value.isEnd() ||
      Boolean(localDraw.value),
  )
  const localDisplay = computed(() =>
    setupPositionAfter(localSetup.value, localMoves.value.slice(0, localPly.value)),
  )
  const localHistory = computed(() => setupSanHistory(localSetup.value, localMoves.value))
  const localLast = computed(() => lastMoveKeys(localMoves.value.slice(0, localPly.value)))
  const localDests = computed(() => setupDests(localSetup.value, localDisplay.value))
  const localTurn = computed(() => turnColor(localDisplay.value))
  const localCheck = computed(() => checkColor(localDisplay.value))
  /** How a finished computer game ended, from the player's point of view. */
  const localResult = computed<{
    kind: 'win' | 'loss' | 'draw'
    title: string
    detail: string
  } | null>(() => {
    if (!localOver.value) return null
    if (resigned.value) return { kind: 'loss', title: 'Stockfish won', detail: 'You resigned' }
    if (flagged.value) {
      const winner = flagged.value === 'white' ? 'black' : 'white'
      // Running out of time is a draw when the other side cannot possibly mate.
      if (localGame.value.hasInsufficientMaterial(winner))
        return { kind: 'draw', title: 'Draw', detail: 'Time ran out, but mate was impossible' }
      const won = winner === userColor.value
      return {
        kind: won ? 'win' : 'loss',
        title: won ? 'You won' : 'Stockfish won',
        detail: won ? 'Stockfish ran out of time' : 'You ran out of time',
      }
    }
    const game = localGame.value
    if (game.isCheckmate()) {
      const won = game.turn !== userColor.value
      return {
        kind: won ? 'win' : 'loss',
        title: won ? 'You won' : 'Stockfish won',
        detail: 'Checkmate',
      }
    }
    const detail = localDraw.value ?? (game.isStalemate() ? 'Stalemate' : 'Insufficient material')
    return { kind: 'draw', title: 'Draw', detail }
  })
  /** The player may touch pieces: it is their move, or they may queue one. */
  const localCanPlay = computed(
    () =>
      allowed() &&
      engineReady.value &&
      !localOver.value &&
      localPly.value === localMoves.value.length,
  )
  const localInteractive = computed(() => localCanPlay.value && localTurn.value === userColor.value)

  /* ── Clock ─────────────────────────────────────────────────────────── */

  /** The clock runs once both sides have moved, as on Lichess. */
  const clockRunning = computed(
    () => allowed() && Boolean(times.value) && !localOver.value && localMoves.value.length >= 2,
  )
  function remaining(color: 'white' | 'black', now = performance.now()): number {
    const left = times.value?.[color] ?? 0
    if (!clockRunning.value || localGame.value.turn !== color) return left
    return Math.max(0, left - (now - turnStarted))
  }
  const localSaveError = persistSession('kchess:computer:v1', () => ({
    version: 2,
    moves: localMoves.value,
    ply: localPly.value,
    level: level.value,
    color: userColor.value,
    resigned: resigned.value,
    setup: localSetup.value,
    clock: localClock.value,
    times: times.value ? { white: remaining('white'), black: remaining('black') } : null,
    flagged: flagged.value,
  }))
  const archive = useGameArchive(
    () => {
      const outcome = localGame.value.outcome()
      const winner = resigned.value
        ? userColor.value === 'white'
          ? 'black'
          : 'white'
        : flagged.value
          ? localGame.value.hasInsufficientMaterial(flagged.value === 'white' ? 'black' : 'white')
            ? undefined
            : flagged.value === 'white'
              ? 'black'
              : 'white'
          : outcome?.winner
      const engine = `Stockfish (${engineLevelInfo(level.value).label})`
      return {
        source: 'computer',
        setup: localSetup.value,
        moves: [...localMoves.value],
        white: userColor.value === 'white' ? 'You' : engine,
        black: userColor.value === 'black' ? 'You' : engine,
        result: localOver.value
          ? winner === 'white'
            ? '1-0'
            : winner === 'black'
              ? '0-1'
              : '1/2-1/2'
          : '*',
        reason: localResult.value?.detail ?? 'In progress',
        timeControl: localClock.value
          ? `${localClock.value.minutes * 60}+${localClock.value.increment}`
          : '-',
      }
    },
    () => localMoves.value.length > 0,
  )

  /** Charge the side that just moved for its thinking and give it its increment. */
  function settleClock(mover: 'white' | 'black'): void {
    const current = times.value
    if (!current) return
    const now = performance.now()
    const spent = clockRunning.value ? now - turnStarted : 0
    const increment = (localClock.value?.increment ?? 0) * 1000
    times.value = {
      ...current,
      [mover]: Math.max(0, current[mover] - spent) + (localMoves.value.length >= 2 ? increment : 0),
    }
    turnStarted = now
  }
  const now = ref(performance.now())
  const ticker = useIntervalFn(
    () => {
      now.value = performance.now()
      if (!clockRunning.value) return
      const turn = localGame.value.turn
      if (remaining(turn) <= 0) {
        times.value = { ...times.value!, [turn]: 0 }
        flagged.value = turn
        gameEpoch.value++
        void window.kchess.stopEngine().catch(() => {})
        thinking.value = false
        void play('lowTime')
      }
    },
    200,
    { immediate: false },
  )
  watch(
    () => allowed() && Boolean(times.value) && !localOver.value,
    (running) => (running ? ticker.resume() : ticker.pause()),
    { immediate: true },
  )
  function clockText(color: 'white' | 'black'): string | undefined {
    if (!times.value) return undefined
    void now.value
    const ms = remaining(color)
    const seconds = Math.ceil(ms / 1000)
    if (ms < 10_000) return `0:${(ms / 1000).toFixed(1).padStart(4, '0')}`
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
  }

  function makeMove(uci: string): void {
    if (localPly.value !== localMoves.value.length || localOver.value || thinking.value) return
    const mover = localGame.value.turn
    const san = playUci(localGame.value.clone(), uci)
    const replayed = san ? null : replaySetup(localSetup.value, [...localMoves.value, uci])
    // Castling may arrive king-two-squares on a Chess960 board; the replay normalises it.
    if (!san && replayed?.played.length !== localMoves.value.length + 1) return
    settleClock(mover)
    localMoves.value = [...localMoves.value, uci]
    localPly.value = localMoves.value.length
    playMoveSound(san || replayed?.played.at(-1)?.san)
    void computerTurn()
  }
  /** The engine's think time: the level's, kept well inside the clock when there is one. */
  function thinkTime(): number | undefined {
    if (!times.value) return undefined
    const base = engineLevelInfo(level.value).time
    const left = remaining(localGame.value.turn)
    const increment = (localClock.value?.increment ?? 0) * 1000
    return Math.max(50, Math.min(5000, base, Math.floor(left / 30 + increment * 0.8)))
  }
  async function computerTurn(): Promise<void> {
    if (
      thinking.value ||
      localGame.value.turn === userColor.value ||
      localOver.value ||
      !engineReady.value ||
      !allowed()
    )
      return
    const epoch = gameEpoch.value,
      moveCount = localMoves.value.length
    thinking.value = true
    try {
      const setup = localSetup.value
      const movetime = thinkTime()
      const move = await window.kchess.bestMove([...localMoves.value], level.value, {
        ...(isStandardStart(setup) ? {} : { fen: setup.fen }),
        ...(setup.variant === 'chess960' ? { chess960: true } : {}),
        ...(movetime ? { movetime } : {}),
      })
      if (epoch !== gameEpoch.value || moveCount !== localMoves.value.length || localOver.value)
        return
      const replayed = replaySetup(setup, [...localMoves.value, move])
      if (replayed?.played.length !== localMoves.value.length + 1) return
      const san = replayed.played.at(-1)!.san
      settleClock(localGame.value.turn)
      localMoves.value = [...localMoves.value, move]
      localPly.value = localMoves.value.length
      playMoveSound(san)
      notifyDesktop('computerMove', 'Stockfish moved', `${san}. Your move.`)
    } catch (cause) {
      // A new game or takeback cancels the search; that is not an error.
      if (epoch === gameEpoch.value) {
        fail(cause)
        // The engine may have vanished since the last check; make the indicator tell the truth.
        void recheckEngine()
      }
    } finally {
      if (epoch === gameEpoch.value) thinking.value = false
    }
  }
  function resetClock(): void {
    const clock = localClock.value
    times.value = clock ? { white: clock.minutes * 60_000, black: clock.minutes * 60_000 } : null
    flagged.value = null
    turnStarted = performance.now()
  }
  /**
   * Start a new game; `setup` and `clock` change the start position and clock for this and
   * later games (omitted, they stay as they were).
   */
  function newGame(setup?: GameSetup, clock?: ComputerClock | null): void {
    // Called from templates too, where the first argument may be an event.
    if (setup && !isVariant((setup as Partial<GameSetup>).variant)) setup = undefined
    archive.reset()
    gameEpoch.value++
    void window.kchess.stopEngine().catch(() => {})
    thinking.value = false
    resigned.value = false
    if (setup) localSetup.value = setup
    if (clock !== undefined) localClock.value = clock
    resetClock()
    localMoves.value = []
    localPly.value = 0
    void computerTurn()
  }
  function takeback(): void {
    if (!localMoves.value.length) return
    gameEpoch.value++
    void window.kchess.stopEngine().catch(() => {})
    thinking.value = false
    resigned.value = false
    flagged.value = null
    // Undo the player's last move and any reply, so it is the player's turn again.
    const kept = localMoves.value.slice(0, -1)
    while (kept.length && setupPositionAfter(localSetup.value, kept).turn !== userColor.value)
      kept.pop()
    localMoves.value = kept
    localPly.value = kept.length
    turnStarted = performance.now()
    // Playing black and undoing back to the start hands the move to the computer.
    void computerTurn()
  }
  /** Give up the game in progress; it stays on the board to review. */
  function resign(): void {
    if (!localMoves.value.length || localOver.value) return
    gameEpoch.value++
    void window.kchess.stopEngine().catch(() => {})
    thinking.value = false
    resigned.value = true
    localPly.value = localMoves.value.length
  }
  function localStatus(): string {
    if (resigned.value) return 'Resigned'
    if (flagged.value) return 'Time out'
    if (localGame.value.isCheckmate()) return 'Checkmate'
    if (localGame.value.isEnd()) return 'Draw'
    if (localDraw.value) return `Draw · ${localDraw.value}`
    if (localGame.value.isCheck()) return 'Check'
    return thinking.value ? 'Computer is thinking…' : `Your move · ${statusText(localGame.value)}`
  }
  return {
    localSaveError,
    localGameEpoch: gameEpoch,
    localMoves,
    localPly,
    level,
    userColor,
    thinking,
    localOver,
    localGame,
    localDisplay,
    localHistory,
    localLast,
    localDests,
    localTurn,
    localCheck,
    localInteractive,
    localCanPlay,
    localResult,
    localSetup,
    localClock,
    computerClockText: clockText,
    newGame,
    takeback,
    resign,
    localStatus,
    makeMove,
  }
}
