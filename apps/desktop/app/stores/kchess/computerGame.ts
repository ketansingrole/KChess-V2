import { computed, reactive, ref, toRefs, watch, onScopeDispose, type Ref } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import type { NotificationKind } from '../../../../../core/src/contracts/types'
import {
  checkColor,
  lastMoveKeys,
  setupDests,
  setupPositionAfter,
  setupSanHistory,
  statusText,
  turnColor,
} from '../../../../../core/src/domain/chess'
import { initialLibrary, persistSession } from '../../utils/library'
import { useGameArchive } from '../gameArchive'
import { isVariant, type GameSetup } from '../../../../../core/src/domain/variant'
import {
  ComputerGame,
  computerState,
  type ComputerClock,
} from '../../../../../core/src/domain/gameSession'
import { playMoveSound, play } from '../../utils/sound'
export type { ComputerClock } from '../../../../../core/src/domain/gameSession'

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
  const saved = initialLibrary().sessions.computer
  const state = reactive(computerState(saved))
  const {
    moves: localMoves,
    ply: localPly,
    level,
    color: userColor,
    setup: localSetup,
    clock: localClock,
    thinking,
    epoch: gameEpoch,
  } = toRefs(state)
  const game = new ComputerGame(state, {
    bestMove: (moves, level, options) => window.kchess.bestMove(moves, level, options),
    stopEngine: () => window.kchess.stopEngine(),
    ready: () => engineReady.value,
    allowed,
    now: () => performance.now(),
    moved(san, computer) {
      playMoveSound(san)
      if (computer) notifyDesktop('computerMove', 'Stockfish moved', `${san}. Your move.`)
    },
    flagged: () => {
      void play('lowTime')
    },
    failed(cause) {
      console.warn('[computer-game] engine operation failed:', cause)
      fail(cause)
      void recheckEngine()
    },
  })
  onScopeDispose(() => game.dispose())
  watch([engineReady, allowed], ([ready, permitted]) => {
    game.availabilityChanged()
    if (ready && permitted && saved) void game.computerTurn()
  })
  const localGame = computed(() => game.position)
  const localDraw = computed(() => game.draw)
  const localOver = computed(() => game.over)
  const localDisplay = computed(() =>
    setupPositionAfter(localSetup.value, localMoves.value.slice(0, localPly.value)),
  )
  const localHistory = computed(() => setupSanHistory(localSetup.value, localMoves.value))
  const localLast = computed(() => lastMoveKeys(localMoves.value.slice(0, localPly.value)))
  const localDests = computed(() => setupDests(localSetup.value, localDisplay.value))
  const localTurn = computed(() => turnColor(localDisplay.value))
  const localCheck = computed(() => checkColor(localDisplay.value))
  const localResult = computed(() => game.result)
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

  const localSaveError = persistSession('computer', () => game.snapshot())
  const archive = useGameArchive(
    () => game.archiveSnapshot(),
    () => localMoves.value.length > 0,
  )

  const now = ref(performance.now())
  const ticker = useIntervalFn(
    () => {
      now.value = performance.now()
      game.expire(now.value)
    },
    200,
    { immediate: false },
  )
  watch(
    () => allowed() && Boolean(state.times) && !localOver.value,
    (running) => (running ? ticker.resume() : ticker.pause()),
    { immediate: true },
  )
  function clockText(color: 'white' | 'black'): string | undefined {
    if (!state.times) return undefined
    void now.value
    const ms = game.remaining(color)
    const seconds = Math.ceil(ms / 1000)
    if (ms < 10_000) return `0:${(ms / 1000).toFixed(1).padStart(4, '0')}`
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
  }

  function makeMove(uci: string): void {
    game.move(uci)
  }
  function newGame(setup?: GameSetup, clock?: ComputerClock | null): void {
    if (setup && !isVariant((setup as Partial<GameSetup>).variant)) setup = undefined
    archive.reset()
    game.start(setup, clock)
  }
  function takeback(): void {
    game.takeback()
  }
  function resign(): void {
    game.resign()
  }
  function localStatus(): string {
    if (state.resigned) return 'Resigned'
    if (state.flagged) return 'Time out'
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
