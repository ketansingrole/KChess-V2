import { computed, ref, type Ref } from 'vue'
import type { EngineLevel, NotificationKind } from '../../../src/shared/types'
import {
  checkColor,
  destsFor,
  drawReason,
  lastMoveKeys,
  playUci,
  positionAfter,
  sanHistory,
  statusText,
  takebackMoves,
  turnColor,
} from '../../utils/chess'
import { playMoveSound } from '../../utils/sound'

/** Computer-game state lives for the store's lifetime, including across route changes. */
export function useComputerGame(options: {
  engineReady: Ref<boolean>
  fail: (cause: unknown) => void
  recheckEngine: () => Promise<void>
  notifyDesktop: (kind: NotificationKind, title: string, body: string) => void
}) {
  const { engineReady, fail, recheckEngine, notifyDesktop } = options
  const localMoves = ref<string[]>([])
  const localPly = ref(0)
  const level = ref<EngineLevel>('medium')
  const userColor = ref<'white' | 'black'>('white')
  const flipped = ref(false)
  const thinking = ref(false)
  const gameEpoch = ref(0)
  const localGame = computed(() => positionAfter(localMoves.value))
  const localDraw = computed(() => drawReason(localMoves.value))
  const localOver = computed(() => localGame.value.isEnd() || Boolean(localDraw.value))
  const localDisplay = computed(() => positionAfter(localMoves.value.slice(0, localPly.value)))
  const localHistory = computed(() => sanHistory(localMoves.value))
  const localLast = computed(() => lastMoveKeys(localMoves.value.slice(0, localPly.value)))
  const localDests = computed(() => destsFor(localDisplay.value))
  const localTurn = computed(() => turnColor(localDisplay.value))
  const localCheck = computed(() => checkColor(localDisplay.value))
  /** How a finished computer game ended, from the player's point of view. */
  const localResult = computed<{
    kind: 'win' | 'loss' | 'draw'
    title: string
    detail: string
  } | null>(() => {
    if (!localOver.value) return null
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
    () => engineReady.value && !localOver.value && localPly.value === localMoves.value.length,
  )
  const localInteractive = computed(() => localCanPlay.value && localTurn.value === userColor.value)

  function makeMove(uci: string): void {
    if (localPly.value !== localMoves.value.length || localOver.value || thinking.value) return
    const san = playUci(positionAfter(localMoves.value), uci)
    if (!san) return
    localMoves.value = [...localMoves.value, uci]
    localPly.value = localMoves.value.length
    playMoveSound(san)
    void computerTurn()
  }
  async function computerTurn(): Promise<void> {
    if (localGame.value.turn === userColor.value || localOver.value || !engineReady.value) return
    const epoch = gameEpoch.value,
      moveCount = localMoves.value.length
    thinking.value = true
    try {
      const move = await window.kchess.bestMove([...localMoves.value], level.value)
      if (epoch !== gameEpoch.value || moveCount !== localMoves.value.length) return
      const san = playUci(positionAfter(localMoves.value), move)
      if (!san) return
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
  function newGame(): void {
    gameEpoch.value++
    thinking.value = false
    localMoves.value = []
    localPly.value = 0
    flipped.value = userColor.value === 'black'
    void computerTurn()
  }
  function takeback(): void {
    if (!localMoves.value.length) return
    gameEpoch.value++
    thinking.value = false
    localMoves.value = takebackMoves(localMoves.value, userColor.value)
    localPly.value = localMoves.value.length
    // Playing black and undoing back to the start hands the move to the computer.
    void computerTurn()
  }
  function localStatus(): string {
    if (localGame.value.isCheckmate()) return 'Checkmate'
    if (localGame.value.isEnd()) return 'Draw'
    if (localDraw.value) return `Draw · ${localDraw.value}`
    if (localGame.value.isCheck()) return 'Check'
    return thinking.value ? 'Computer is thinking…' : `Your move · ${statusText(localGame.value)}`
  }
  return {
    localMoves,
    localPly,
    level,
    userColor,
    flipped,
    thinking,
    localOver,
    localDisplay,
    localHistory,
    localLast,
    localDests,
    localTurn,
    localCheck,
    localInteractive,
    localCanPlay,
    localResult,
    newGame,
    takeback,
    localStatus,
    makeMove,
  }
}
