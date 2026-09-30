import { computed, ref, watch } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import type {
  ChallengeColor,
  NotificationKind,
  OnlineEvent,
  PresenceReport,
} from '../../../src/shared/types'
import type { PlayerPresence } from '../../components/PlayerLine.vue'
import { isGameInProgress } from '../../../src/shared/gameStatus'
import {
  checkColor,
  destsFor,
  lastMoveKeys,
  positionAfter,
  sanHistory,
  turnColor,
} from '../../utils/chess'
import { Clock, formatClock } from '../../utils/clock'
import { signalFromLatency } from '../../utils/format'
import { play, playMoveSound } from '../../utils/sound'

/** Live game, stream lifecycle, presence and clocks share a single owner across routes. */
export function useOnlineGame(options: {
  activeAccount: () => string
  run: <T>(action: () => Promise<T>) => Promise<T | undefined>
  fail: (cause: unknown) => void
  notifyDesktop: (kind: NotificationKind, title: string, body: string) => void
}) {
  const { activeAccount, run, fail, notifyDesktop } = options
  const onlinePhase = ref<'idle' | 'seeking' | 'playing' | 'finished'>('idle')
  const onlineId = ref('')
  const onlineMoves = ref<string[]>([])
  const onlineColor = ref<'white' | 'black'>('white')
  const onlineOpponent = ref('Opponent')
  const onlineStatus = ref('')
  const onlineInitial = ref(0)
  const onlineMinutes = ref(15)
  const onlineIncrement = ref(10)
  const onlineTarget = ref('')
  const onlineChoice = ref<ChallengeColor>('random')
  /** The "rated" choice in the find-a-game form. */
  const onlineRated = ref(false)
  /** Whether the game being played is rated, as Lichess reports it. */
  const onlineGameRated = ref(false)
  /** Position being looked at while reviewing a live game; null follows the game. */
  const onlineViewPly = ref<number | null>(null)
  const onlineFlipped = ref(false)
  const opponentId = ref('')
  const opponentGone = ref(false)
  const presence = ref<PresenceReport | null>(null)
  /** Recent round trips to Lichess, oldest first; the game panel graphs them. */
  const latencySamples = ref<number[]>([])
  /** How long Lichess took to accept our last move. */
  const moveAckMs = ref<number | null>(null)
  const clock = new Clock()
  /** Drives the on-screen clocks and the low-time alert; runs only while the app is initialised. */
  const ticker = useIntervalFn(
    () => {
      now.value = performance.now()
      if (onlinePhase.value === 'playing') {
        const turn = onlineTurn.value
        if (clock.lowTimeAlert(turn)) {
          void play('lowTime')
          if (turn === onlineColor.value)
            notifyDesktop(
              'lowTime',
              'Low on time',
              `${formatClock(clock.remaining(turn))} left against ${onlineOpponent.value}.`,
            )
        }
      }
    },
    250,
    { immediate: false },
  )

  const onlinePosition = computed(() => positionAfter(onlineMoves.value))
  const onlineHistory = computed(() => sanHistory(onlineMoves.value))
  const onlinePly = computed(() =>
    Math.min(onlineViewPly.value ?? Infinity, onlineMoves.value.length),
  )
  const onlineAtLive = computed(() => onlinePly.value === onlineMoves.value.length)
  /** The position on the board: the live one, or an earlier one while reviewing. */
  const onlineDisplay = computed(() =>
    onlineAtLive.value
      ? onlinePosition.value
      : positionAfter(onlineMoves.value.slice(0, onlinePly.value)),
  )
  const onlineLast = computed(() => lastMoveKeys(onlineMoves.value.slice(0, onlinePly.value)))
  const onlineDests = computed(() => destsFor(onlinePosition.value))
  /** Whose turn it really is (drives the clocks); the board's turn is `onlineDisplayTurn`. */
  const onlineTurn = computed(() => turnColor(onlinePosition.value))
  const onlineDisplayTurn = computed(() => turnColor(onlineDisplay.value))
  const onlineCheck = computed(() => checkColor(onlineDisplay.value))
  /** The player may touch pieces: it is their move, or they may queue one. */
  const onlineCanPlay = computed(() => onlinePhase.value === 'playing' && onlineAtLive.value)
  const onlineInteractive = computed(
    () => onlineCanPlay.value && onlineTurn.value === onlineColor.value,
  )
  function viewOnlinePly(ply: number): void {
    const clamped = Math.max(0, Math.min(ply, onlineMoves.value.length))
    onlineViewPly.value = clamped === onlineMoves.value.length ? null : clamped
  }
  const onlineOrientation = computed(() =>
    onlineFlipped.value ? (onlineColor.value === 'white' ? 'black' : 'white') : onlineColor.value,
  )

  /** Lichess's view of each side's connection, for the player lines. */
  const onlinePresence = computed<{ self?: PlayerPresence; opponent?: PlayerPresence }>(() => {
    if (onlinePhase.value !== 'playing') return {}
    const report = presence.value
    const me = report?.users[activeAccount().toLowerCase()]
    const them = report?.users[opponentId.value.toLowerCase()]
    const ping = pingStats.value
    return {
      self: ping
        ? {
            state: me?.online === false ? 'offline' : 'online',
            signal: me?.signal ?? signalFromLatency(ping.current),
            latencyMs: ping.current,
          }
        : { state: 'checking', label: 'Measuring…' },
      opponent: opponentGone.value
        ? { state: 'away', signal: undefined }
        : them
          ? { state: them.online ? 'online' : 'offline', signal: them.signal }
          : { state: 'checking', label: 'Checking…' },
    }
  })
  /** Summary of the recent round trips to Lichess, or null before the first one. */
  const pingStats = computed(() => {
    const samples = latencySamples.value
    if (!samples.length) return null
    const current = samples[samples.length - 1]!
    const average = Math.round(samples.reduce((sum, ms) => sum + ms, 0) / samples.length)
    return {
      current,
      average,
      min: Math.min(...samples),
      max: Math.max(...samples),
      samples,
      quality: signalFromLatency(current),
    }
  })
  /** How often the connection is measured while a game is on; Lichess asks for about once per 5 s at most per check, this stays under that. */
  const PRESENCE_INTERVAL_MS = 3000
  async function pollPresence(): Promise<void> {
    const me = activeAccount()
    if (onlinePhase.value !== 'playing' || !me) return
    try {
      // Measure even before the opponent is known, so the ping shows from the first second.
      const ids = opponentId.value ? [me, opponentId.value] : [me]
      const report = await window.kchess.presence(ids)
      presence.value = report
      latencySamples.value = [...latencySamples.value, report.latencyMs].slice(-30)
    } catch {
      // A missed poll just leaves the last reading up; the next one recovers.
    }
  }
  const presenceTicker = useIntervalFn(() => void pollPresence(), PRESENCE_INTERVAL_MS, {
    immediate: false,
  })
  watch(
    onlinePhase,
    (phase) => {
      if (phase === 'playing') {
        presenceTicker.resume()
        void pollPresence()
      } else {
        presenceTicker.pause()
        presence.value = null
        latencySamples.value = []
        moveAckMs.value = null
        opponentGone.value = false
      }
    },
    { immediate: true },
  )
  watch(opponentId, () => void pollPresence())

  let onlineRequest = 0
  async function startOnline(): Promise<void> {
    if (onlinePhase.value === 'playing' || onlinePhase.value === 'seeking') return
    const request = ++onlineRequest
    onlineId.value = ''
    onlinePhase.value = 'seeking'
    onlineStatus.value = 'Looking for an opponent…'
    const result = await run(async () => {
      try {
        return await window.kchess.startOnline({
          minutes: onlineMinutes.value,
          increment: onlineIncrement.value,
          color: onlineChoice.value,
          rated: onlineRated.value,
          target: onlineTarget.value || undefined,
          account: activeAccount() || undefined,
        })
      } catch (cause) {
        if (request === onlineRequest) throw cause
      }
    })
    if (request !== onlineRequest || onlinePhase.value !== 'seeking') return
    if (result) {
      onlineStatus.value = result.url
        ? 'Challenge sent. Waiting for acceptance…'
        : 'Looking for an opponent…'
    } else {
      onlinePhase.value = 'idle'
      onlineStatus.value = ''
    }
  }
  async function stopOnline(): Promise<void> {
    ++onlineRequest
    onlinePhase.value = 'idle'
    onlineStatus.value = ''
    onlineViewPly.value = null
    await window.kchess.cancelOnline().catch(fail)
  }
  function readOnlineEvent(event: OnlineEvent): void {
    if (event.type === 'gameStart') {
      onlineId.value = event.game.gameId
      onlineColor.value = event.game.color ?? 'white'
      onlineOpponent.value = event.game.opponent?.username ?? 'Opponent'
      onlineMoves.value = []
      onlineViewPly.value = null
      onlineFlipped.value = false
      onlineGameRated.value = event.game.rated ?? false
      opponentId.value = event.game.opponent?.id ?? ''
      opponentGone.value = false
      presence.value = null
      onlineInitial.value = 0
      onlinePhase.value = 'playing'
      onlineStatus.value = 'Game in progress'
      notifyDesktop(
        'gameEvents',
        'Game started',
        `You are playing ${onlineOpponent.value} as ${onlineColor.value}.`,
      )
      return
    }
    if (event.type === 'gameFinish') {
      if (event.game.gameId !== onlineId.value) return
      const wasPlaying = onlinePhase.value === 'playing'
      const reason = event.game.status?.name ?? 'finished'
      onlinePhase.value = 'finished'
      onlineStatus.value = `Game ended: ${reason}`
      if (wasPlaying) {
        const winner = event.game.winner
        const outcome =
          reason === 'aborted'
            ? 'Game aborted'
            : !winner
              ? 'Draw'
              : winner === (event.game.color ?? onlineColor.value)
                ? 'You won'
                : 'You lost'
        notifyDesktop('gameEvents', outcome, `Against ${onlineOpponent.value} · ${reason}.`)
      }
      return
    }
    if (event.type === 'opponentGone') {
      if ('id' in event && event.id !== onlineId.value) return
      if (onlinePhase.value === 'playing') {
        opponentGone.value = event.gone
        onlineStatus.value = event.gone
          ? event.claimWinInSeconds
            ? `Opponent disconnected… you can claim the win in ${event.claimWinInSeconds}s`
            : 'Opponent disconnected…'
          : 'Game in progress'
      }
      return
    }
    // Challenges and chat lines carry no game state; treating them as one would wipe the board.
    if (event.type !== 'gameFull' && event.type !== 'gameState') return
    if ('id' in event && onlineId.value && event.id !== onlineId.value) return
    const state = event.type === 'gameFull' ? event.state : event
    const previousCount = onlineMoves.value.length
    if (event.type === 'gameFull') {
      onlineId.value = event.id
      onlineInitial.value = (event.clock?.initial ?? 0) / 1000
      onlineGameRated.value = event.rated
      const ours = activeAccount().toLowerCase()
      // Match by account; if neither side does, keep the colour `gameStart` announced.
      if (event.white.id?.toLowerCase() === ours) onlineColor.value = 'white'
      else if (event.black.id?.toLowerCase() === ours) onlineColor.value = 'black'
      const opponent = onlineColor.value === 'white' ? event.black : event.white
      onlineOpponent.value = opponent.name ?? 'Opponent'
      opponentId.value = opponent.id ?? ''
    }
    onlineMoves.value = String(state.moves ?? '')
      .trim()
      .split(/\s+/)
      .filter(Boolean)
    if (previousCount && onlineMoves.value.length > previousCount) {
      const san = sanHistory(onlineMoves.value).at(-1)
      playMoveSound(san)
      // Even plies are white's moves; a move by the other colour is the opponent's.
      const mover = (onlineMoves.value.length - 1) % 2 === 0 ? 'white' : 'black'
      if (mover !== onlineColor.value)
        notifyDesktop('opponentMove', `${onlineOpponent.value} moved`, `${san}. Your move.`)
    }
    const running = isGameInProgress(state.status)
    clock.set({
      white: Number(state.wtime ?? 0),
      black: Number(state.btime ?? 0),
      ticking: running
        ? onlineMoves.value.length
          ? turnColor(positionAfter(onlineMoves.value))
          : 'white'
        : undefined,
      initialSeconds: onlineInitial.value || undefined,
    })
    if (running) {
      onlinePhase.value = 'playing'
      onlineStatus.value = 'Game in progress'
    } else {
      onlinePhase.value = 'finished'
      onlineStatus.value = `Game ended: ${state.status}`
    }
  }
  async function onlineMove(uci: string): Promise<void> {
    if (!onlineId.value || onlinePhase.value !== 'playing') return
    const sent = performance.now()
    try {
      await window.kchess.playOnline(onlineId.value, uci)
      moveAckMs.value = Math.round(performance.now() - sent)
    } catch (cause) {
      fail(cause)
    }
  }
  async function onlineAction(
    action: 'resign' | 'abort' | 'takeback' | 'declineTakeback',
  ): Promise<void> {
    if (!onlineId.value) return
    await run(() => window.kchess.onlineAction(onlineId.value, action))
  }
  function clockText(color: 'white' | 'black'): string {
    void now.value
    return formatClock(clock.remaining(color))
  }
  const now = ref(performance.now())
  return {
    onlinePhase,
    onlineId,
    onlineMoves,
    onlineColor,
    onlineOpponent,
    onlineStatus,
    onlineMinutes,
    onlineIncrement,
    onlineTarget,
    onlineChoice,
    onlineRated,
    onlineGameRated,
    onlineFlipped,
    presence,
    moveAckMs,
    ticker,
    presenceTicker,
    onlinePosition,
    onlineHistory,
    onlinePly,
    onlineDisplay,
    onlineLast,
    onlineDests,
    onlineTurn,
    onlineDisplayTurn,
    onlineCheck,
    onlineCanPlay,
    onlineInteractive,
    viewOnlinePly,
    onlineOrientation,
    onlinePresence,
    pingStats,
    PRESENCE_INTERVAL_MS,
    startOnline,
    stopOnline,
    readOnlineEvent,
    onlineMove,
    onlineAction,
    clockText,
  }
}
