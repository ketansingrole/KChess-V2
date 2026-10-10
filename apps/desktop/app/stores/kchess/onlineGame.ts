import { computed, reactive, ref, toRefs, watch } from 'vue'
import { useIntervalFn, useTimeoutPoll } from '@vueuse/core'
import type {
  ChallengeColor,
  ChatRoom,
  CorrespondenceDays,
  NotificationKind,
  OnlineAction,
  OnlineEvent,
  OnlineConnection,
  OnlineOptions,
  PresenceReport,
} from '@kchess/contracts/types'
import type { PlayerPresence } from '../../components/PlayerLine.vue'
import { OnlineGame, onlineGameState } from '@kchess/rules/onlineGame'
import type { Variant } from '@kchess/rules/variant'
import {
  checkColor,
  lastMoveKeys,
  setupDests,
  setupPositionAfter,
  setupSanHistory,
  turnColor,
} from '@kchess/rules/chess'
import { formatClock } from '@kchess/rules/clock'
import { signalFromLatency } from '../../utils/format'
import { play, playMoveSound } from '../../utils/sound'
export type { FinishedGame } from '@kchess/rules/onlineGame'

/** Live game, stream lifecycle, presence and clocks share a single owner across routes. */
export function useOnlineGame(options: {
  activeAccount: () => string
  run: <T>(action: () => Promise<T>) => Promise<T | undefined>
  fail: (cause: unknown) => void
  info: (text: string) => void
  notifyDesktop: (kind: NotificationKind, title: string, body: string) => void
  chatEnabled: () => boolean
}) {
  const { activeAccount, run, fail, info, notifyDesktop, chatEnabled } = options
  let presenceEpoch = 0
  let presencePending = false
  /* The find-a-game form. */
  const onlineMinutes = ref(15)
  const onlineIncrement = ref(10)
  const onlineTarget = ref('')
  const onlineChoice = ref<ChallengeColor>('random')
  /** The "rated" choice in the find-a-game form. */
  const onlineRated = ref(false)
  const onlineVariant = ref<Variant>('standard')
  /** Real-time with a clock, or correspondence with days per move. */
  const onlineMode = ref<'realtime' | 'correspondence'>('realtime')
  const onlineDays = ref<CorrespondenceDays>(3)
  /* The game on the board. */
  /** Whether the game being played is rated, as Lichess reports it. */
  /** Rules and start of the game on the board. */
  /** Lichess's variant key when KChess cannot show that variant (Crazyhouse). */
  /** Position being looked at while reviewing a live game; null follows the game. */
  const onlineViewPly = ref<number | null>(null)
  const onlineFlipped = ref(false)
  /** When the win can be claimed (performance.now() time), while the opponent is gone. */
  /** Draw and takeback offers on the table: ours, theirs, or none. */
  /** Abort deadline for the first move (performance.now() time). */
  const presence = ref<PresenceReport | null>(null)
  /** Recent round trips to Lichess, oldest first; the game panel graphs them. */
  const latencySamples = ref<number[]>([])
  /** How long Lichess took to accept our last move. */
  const moveAckMs = ref<number | null>(null)
  const state = reactive(onlineGameState())
  const {
    onlinePhase,
    onlineId,
    onlineAccount,
    onlineConnection,
    onlineMoves,
    onlineColor,
    onlineOpponent,
    onlineOpponentRating,
    onlineStatus,
    onlineGameRated,
    onlineSetup,
    onlineUnsupported,
    onlineSpeed,
    onlineClockConfig,
    onlineTournament,
    opponentId,
    opponentGone,
    claimAt,
    drawOffer,
    takebackOffer,
    firstMoveBy,
    berserked,
    chat,
    lastGame,
  } = toRefs(state)
  const controller = new OnlineGame(state, {
    api: window.kchess,
    activeAccount,
    chatEnabled,
    now: () => performance.now(),
    notify: notifyDesktop,
    moved: playMoveSound,
    failed: fail,
    resetView() {
      onlineViewPly.value = null
      onlineFlipped.value = false
      presence.value = null
    },
    connectionChanged() {
      presenceEpoch++
    },
    moveAcknowledged(ms) {
      moveAckMs.value = ms
    },
  })
  const clock = controller.clock
  const readOnlineState = (state: OnlineConnection): void => controller.readOnlineState(state)
  const readOnlineEvent = (event: OnlineEvent): void => controller.readOnlineEvent(event)
  const checkingOnline = (): void => controller.checkingOnline()
  const onlineMove = (uci: string): Promise<void> => controller.onlineMove(uci)
  const onlineAction = (action: OnlineAction): Promise<void> => controller.onlineAction(action)
  const now = ref(performance.now())
  /** Drives the on-screen clocks and the low-time alert; runs only while the app is initialised. */
  const ticker = useIntervalFn(
    () => {
      now.value = performance.now()
      if (onlinePhase.value === 'playing' && onlineClockConfig.value) {
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

  const onlinePosition = computed(() => setupPositionAfter(onlineSetup.value, onlineMoves.value))
  const onlineHistory = computed(() => setupSanHistory(onlineSetup.value, onlineMoves.value))
  const onlinePly = computed(() =>
    Math.min(onlineViewPly.value ?? Infinity, onlineMoves.value.length),
  )
  const onlineAtLive = computed(() => onlinePly.value === onlineMoves.value.length)
  /** The position on the board: the live one, or an earlier one while reviewing. */
  const onlineDisplay = computed(() =>
    onlineAtLive.value
      ? onlinePosition.value
      : setupPositionAfter(onlineSetup.value, onlineMoves.value.slice(0, onlinePly.value)),
  )
  const onlineLast = computed(() => lastMoveKeys(onlineMoves.value.slice(0, onlinePly.value)))
  const onlineDests = computed(() => setupDests(onlineSetup.value, onlinePosition.value))
  /** Whose turn it really is (drives the clocks); the board's turn is `onlineDisplayTurn`. */
  const onlineTurn = computed(() => turnColor(onlinePosition.value))
  const onlineDisplayTurn = computed(() => turnColor(onlineDisplay.value))
  const onlineCheck = computed(() => checkColor(onlineDisplay.value))
  /** The player may touch pieces: it is their move, or they may queue one. */
  const onlineCanPlay = computed(
    () => onlinePhase.value === 'playing' && onlineAtLive.value && !onlineUnsupported.value,
  )
  const onlineInteractive = computed(
    () => onlineCanPlay.value && onlineTurn.value === onlineColor.value,
  )
  const onlineCorrespondence = computed(() => controller.correspondence)
  /** Moves this side has made so far. */
  /** Arena games can be berserked (half the clock, no increment) before your first move. */
  const canBerserk = computed(
    () =>
      onlinePhase.value === 'playing' &&
      Boolean(onlineTournament.value) &&
      Boolean(onlineClockConfig.value) &&
      !berserked.value &&
      controller.ownMoves === 0,
  )
  /** Seconds until the win can be claimed, or 0 once it can; null when the opponent is here. */
  const claimIn = computed(() => {
    void now.value
    if (!opponentGone.value || claimAt.value === null) return null
    return Math.max(0, Math.ceil((claimAt.value - performance.now()) / 1000))
  })
  /** Seconds left to make the first move before Lichess aborts the game. */
  const firstMoveIn = computed(() => {
    void now.value
    if (firstMoveBy.value === null || onlinePhase.value !== 'playing') return null
    return Math.max(0, Math.ceil((firstMoveBy.value - performance.now()) / 1000))
  })
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
    const me = report?.users[(onlineAccount.value || activeAccount()).toLowerCase()]
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
  /** One request at a time, with stale responses rejected after a phase/session change. */
  const PRESENCE_INTERVAL_MS = 5000
  async function pollPresence(): Promise<void> {
    const me = onlineAccount.value || activeAccount()
    // A correspondence game moves once a day; its presence is not worth polling.
    if (presencePending || onlinePhase.value !== 'playing' || !me || onlineCorrespondence.value)
      return
    const epoch = presenceEpoch
    presencePending = true
    try {
      // Measure even before the opponent is known, so the ping shows from the first second.
      const ids = opponentId.value ? [me, opponentId.value] : [me]
      const report = await window.kchess.presence(ids)
      if (epoch !== presenceEpoch || onlinePhase.value !== 'playing') return
      presence.value = report
      latencySamples.value = [...latencySamples.value, report.latencyMs].slice(-30)
    } catch (cause) {
      console.warn('[online-game] polling presence failed:', cause)
      // A missed poll leaves the last reading; a later tick recovers.
    } finally {
      presencePending = false
    }
  }
  const presenceTicker = useTimeoutPoll(pollPresence, PRESENCE_INTERVAL_MS, {
    immediate: false,
  })
  watch(
    onlinePhase,
    (phase) => {
      presenceEpoch++
      if (phase === 'playing') {
        presenceTicker.resume()
        void pollPresence()
      } else {
        presenceTicker.pause()
        presence.value = null
        latencySamples.value = []
        moveAckMs.value = null
        opponentGone.value = false
        claimAt.value = null
        firstMoveBy.value = null
      }
    },
    { immediate: true },
  )
  watch(opponentId, () => void pollPresence())

  /** Everything about one game, forgotten when another takes the board. */

  /** The form's game, or `override` (a rematch). */
  async function startOnline(override?: Partial<OnlineOptions>): Promise<void> {
    const correspondence =
      override?.days !== undefined || (!override && onlineMode.value === 'correspondence')
    const wanted: OnlineOptions = {
      minutes: onlineMinutes.value,
      increment: onlineIncrement.value,
      color: onlineChoice.value,
      rated: onlineRated.value,
      target: onlineTarget.value.trim() || undefined,
      account: onlineAccount.value || activeAccount() || undefined,
      variant: onlineVariant.value === 'standard' ? undefined : onlineVariant.value,
      days: correspondence && !override ? onlineDays.value : undefined,
      ...override,
    }
    const started = await run(() => controller.start(wanted))
    if (started && correspondence)
      info(
        wanted.target
          ? `Correspondence challenge sent to @${wanted.target}. It appears under your games once accepted.`
          : 'Correspondence seek created on Lichess. It stays open there until someone joins.',
      )
  }

  async function stopOnline(): Promise<void> {
    onlineViewPly.value = null
    await controller.stop().catch(fail)
  }

  /** Offer the last opponent the same game again, colours swapped, as Lichess's rematch does. */
  async function rematch(): Promise<void> {
    const options = controller.rematchOptions()
    if (options) await startOnline(options)
  }

  async function reconnectOnline(): Promise<void> {
    await run(() => controller.reconnect())
  }

  /** Open one of the account's ongoing games (a correspondence game) on the board. */
  async function openOngoing(account: string, id: string): Promise<void> {
    await run(() => controller.open(account, id))
  }

  async function sendChat(text: string, room: ChatRoom = 'player'): Promise<boolean> {
    if (!onlineId.value || !text.trim()) return false
    const sent = await run(async () => {
      await window.kchess.sendChat(onlineId.value, room, text.trim())
      return true
    })
    return Boolean(sent)
  }

  function clockText(color: 'white' | 'black'): string {
    void now.value
    return formatClock(clock.remaining(color))
  }
  return {
    onlinePhase,
    onlineAccount,
    onlineConnection,
    readOnlineState,
    reconnectOnline,
    checkingOnline,
    openOngoing,
    onlineId,
    onlineMoves,
    onlineColor,
    onlineOpponent,
    onlineOpponentRating,
    onlineStatus,
    onlineMinutes,
    onlineIncrement,
    onlineTarget,
    onlineChoice,
    onlineRated,
    onlineVariant,
    onlineMode,
    onlineDays,
    onlineGameRated,
    onlineSetup,
    onlineUnsupported,
    onlineSpeed,
    onlineClockConfig,
    onlineCorrespondence,
    onlineTournament,
    onlineFlipped,
    opponentId,
    drawOffer,
    takebackOffer,
    claimIn,
    firstMoveIn,
    canBerserk,
    berserked,
    chat,
    sendChat,
    lastGame,
    rematch,
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
