import { computed, ref, watch } from 'vue'
import { useIntervalFn, useTimeoutPoll } from '@vueuse/core'
import type {
  ChallengeColor,
  ChatLine,
  ChatRoom,
  CorrespondenceDays,
  NotificationKind,
  OnlineAction,
  OnlineEvent,
  OnlineConnection,
  OnlineOptions,
  PresenceReport,
} from '../../../src/shared/types'
import type { PlayerPresence } from '../../components/PlayerLine.vue'
import { isGameInProgress } from '../../../src/shared/gameStatus'
import {
  defaultFen,
  STANDARD_SETUP,
  variantFromLichess,
  type GameSetup,
  type Variant,
} from '../../../src/shared/variant'
import {
  checkColor,
  lastMoveKeys,
  setupDests,
  setupPositionAfter,
  setupSanHistory,
  turnColor,
} from '../../utils/chess'
import { Clock, formatClock } from '../../utils/clock'
import { signalFromLatency } from '../../utils/format'
import { play, playMoveSound } from '../../utils/sound'

/** What the last game was, so a rematch can offer the same again with colours swapped. */
export interface FinishedGame {
  id: string
  account: string
  opponent: string
  color: 'white' | 'black'
  rated: boolean
  setup: GameSetup
  minutes?: number
  increment?: number
  days?: CorrespondenceDays
}

const CORRESPONDENCE: readonly number[] = [1, 2, 3, 5, 7, 10, 14]

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
  const onlinePhase = ref<'idle' | 'seeking' | 'playing' | 'finished' | 'disconnected'>('idle')
  const onlineId = ref('')
  const onlineAccount = ref('')
  const onlineConnection = ref<OnlineConnection | null>(null)
  let presenceEpoch = 0
  let presencePending = false
  let stateEpoch = -1
  const onlineMoves = ref<string[]>([])
  const onlineColor = ref<'white' | 'black'>('white')
  const onlineOpponent = ref('Opponent')
  const onlineOpponentRating = ref<number | undefined>()
  const onlineStatus = ref('')
  const onlineInitial = ref(0)
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
  const onlineGameRated = ref(false)
  /** Rules and start of the game on the board. */
  const onlineSetup = ref<GameSetup>(STANDARD_SETUP)
  /** Lichess's variant key when KChess cannot show that variant (Crazyhouse). */
  const onlineUnsupported = ref('')
  const onlineSpeed = ref('')
  const onlineClockConfig = ref<{ initial: number; increment: number } | null>(null)
  const onlineDaysPerTurn = ref<number | undefined>()
  const onlineTournament = ref('')
  /** Position being looked at while reviewing a live game; null follows the game. */
  const onlineViewPly = ref<number | null>(null)
  const onlineFlipped = ref(false)
  const opponentId = ref('')
  const opponentGone = ref(false)
  /** When the win can be claimed (performance.now() time), while the opponent is gone. */
  const claimAt = ref<number | null>(null)
  /** Draw and takeback offers on the table: ours, theirs, or none. */
  const drawOffer = ref<'none' | 'mine' | 'theirs'>('none')
  const takebackOffer = ref<'none' | 'mine' | 'theirs'>('none')
  /** Abort deadline for the first move (performance.now() time). */
  const firstMoveBy = ref<number | null>(null)
  const berserked = ref(false)
  const chat = ref<ChatLine[]>([])
  const lastGame = ref<FinishedGame | null>(null)
  const presence = ref<PresenceReport | null>(null)
  /** Recent round trips to Lichess, oldest first; the game panel graphs them. */
  const latencySamples = ref<number[]>([])
  /** How long Lichess took to accept our last move. */
  const moveAckMs = ref<number | null>(null)
  const clock = new Clock()
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
  const onlineCorrespondence = computed(
    () => onlineSpeed.value === 'correspondence' || onlineDaysPerTurn.value !== undefined,
  )
  /** Moves this side has made so far. */
  const ownMoves = computed(() => {
    const whiteFirst = onlineSetup.value.fen.split(' ')[1] !== 'b'
    const mine = onlineColor.value === 'white' ? 0 : 1
    return onlineMoves.value.filter((_, i) => (whiteFirst ? i % 2 : (i + 1) % 2) === mine).length
  })
  /** Arena games can be berserked (half the clock, no increment) before your first move. */
  const canBerserk = computed(
    () =>
      onlinePhase.value === 'playing' &&
      Boolean(onlineTournament.value) &&
      Boolean(onlineClockConfig.value) &&
      !berserked.value &&
      ownMoves.value === 0,
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
  function resetGame(): void {
    onlineMoves.value = []
    onlineViewPly.value = null
    onlineFlipped.value = false
    opponentGone.value = false
    claimAt.value = null
    drawOffer.value = 'none'
    takebackOffer.value = 'none'
    firstMoveBy.value = null
    berserked.value = false
    chat.value = []
    presence.value = null
    onlineInitial.value = 0
    onlineSetup.value = STANDARD_SETUP
    onlineUnsupported.value = ''
    onlineSpeed.value = ''
    onlineClockConfig.value = null
    onlineDaysPerTurn.value = undefined
    onlineTournament.value = ''
    onlineOpponentRating.value = undefined
  }
  let shownGame = ''

  let onlineRequest = 0
  /** The form's game, or `override` (a rematch). */
  async function startOnline(override?: Partial<OnlineOptions>): Promise<void> {
    if (onlinePhase.value === 'disconnected' && onlineId.value) return
    if (onlinePhase.value === 'playing' && !onlineCorrespondence.value) return
    if (onlinePhase.value === 'seeking') return
    const request = ++onlineRequest
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
    const previousPhase = onlinePhase.value
    if (!correspondence) {
      onlineId.value = ''
      onlineAccount.value = wanted.account ?? activeAccount()
      onlinePhase.value = 'seeking'
      onlineStatus.value = 'Looking for an opponent…'
    }
    const result = await run(async () => {
      try {
        return await window.kchess.startOnline(wanted)
      } catch (cause) {
        if (request === onlineRequest) throw cause
      }
    })
    if (request !== onlineRequest) return
    if (correspondence) {
      if (result)
        info(
          wanted.target
            ? `Correspondence challenge sent to @${wanted.target}. It appears under your games once accepted.`
            : 'Correspondence seek created on Lichess. It stays open there until someone joins.',
        )
      return
    }
    if (onlinePhase.value !== 'seeking') return
    if (result) {
      onlineStatus.value = result.url
        ? 'Challenge sent. Waiting for acceptance…'
        : 'Looking for an opponent…'
    } else {
      onlinePhase.value = previousPhase === 'finished' ? 'finished' : 'idle'
      onlineStatus.value = ''
    }
  }
  async function stopOnline(): Promise<void> {
    ++onlineRequest
    if (onlinePhase.value !== 'disconnected' && onlinePhase.value !== 'playing') {
      onlinePhase.value = 'idle'
      onlineStatus.value = ''
    }
    onlineViewPly.value = null
    await window.kchess.cancelOnline().catch(fail)
  }
  /** Offer the last opponent the same game again, colours swapped, as Lichess's rematch does. */
  async function rematch(): Promise<void> {
    const game = lastGame.value
    if (!game) return
    await startOnline({
      target: game.opponent,
      account: game.account,
      color: game.color === 'white' ? 'black' : 'white',
      rated: game.rated,
      minutes: game.minutes ?? onlineMinutes.value,
      increment: game.increment ?? onlineIncrement.value,
      days: game.days,
      variant: game.setup.variant === 'standard' ? undefined : game.setup.variant,
      fen:
        game.setup.variant === 'standard' && game.setup.fen !== STANDARD_SETUP.fen
          ? game.setup.fen
          : undefined,
    })
  }
  function readOnlineState(state: OnlineConnection): void {
    if (state.session < stateEpoch) return
    if (state.session !== stateEpoch) {
      stateEpoch = state.session
      presenceEpoch++
    }
    onlineConnection.value = state
    if (state.account) onlineAccount.value = state.account
    if (state.gameId) onlineId.value = state.gameId
    if (state.lane === 'events' && onlineId.value) return
    if (state.lane === 'game' && state.phase === 'idle') {
      onlinePhase.value = 'idle'
      onlineId.value = ''
      onlineStatus.value = 'No game in progress.'
      clock.pause()
    }
    if (['checking', 'reconnecting', 'disconnected', 'auth-required'].includes(state.phase)) {
      onlinePhase.value = 'disconnected'
      onlineStatus.value =
        state.message ?? 'Connection interrupted. Reconnect to recover your game.'
      clock.pause()
    }
  }
  function checkingOnline(): void {
    onlinePhase.value = 'disconnected'
    onlineStatus.value = 'Checking Lichess for an ongoing game…'
    onlineConnection.value = {
      session: stateEpoch,
      account: onlineAccount.value,
      gameId: onlineId.value,
      lane: 'game',
      phase: 'checking',
      message: onlineStatus.value,
    }
    clock.pause()
  }
  async function reconnectOnline(): Promise<void> {
    checkingOnline()
    const resumed = await run(() => window.kchess.resumeOnline())
    if (resumed) {
      onlineAccount.value = resumed.account
      onlineId.value = resumed.id
      onlineStatus.value = 'Reconnecting…'
    } else if (resumed === null) {
      onlinePhase.value = 'idle'
      onlineStatus.value = 'No game in progress. You can find another game.'
    }
  }
  /** Open one of the account's ongoing games (a correspondence game) on the board. */
  async function openOngoing(account: string, id: string): Promise<void> {
    const opened = await run(async () => {
      await window.kchess.openGame(account, id)
      return true
    })
    if (!opened) return
    if (shownGame !== id) resetGame()
    shownGame = ''
    onlineAccount.value = account
    onlineId.value = id
    onlineStatus.value = 'Opening game…'
    if (onlinePhase.value !== 'playing') onlinePhase.value = 'disconnected'
  }
  async function loadChat(id: string): Promise<void> {
    if (!chatEnabled()) return
    try {
      const lines = await window.kchess.onlineChat(id)
      if (onlineId.value === id)
        chat.value = [...lines, ...chat.value.filter((l) => l.room !== 'player')]
    } catch (cause) {
      console.warn('[online-game] loading chat failed:', cause)
      // The chat is a courtesy; the game goes on without it.
    }
  }
  async function sendChat(text: string, room: ChatRoom = 'player'): Promise<boolean> {
    if (!onlineId.value || !text.trim()) return false
    const sent = await run(async () => {
      await window.kchess.sendChat(onlineId.value, room, text.trim())
      return true
    })
    return Boolean(sent)
  }
  function finish(winner: 'white' | 'black' | undefined, reason: string, notify: boolean): void {
    const previous = onlinePhase.value
    onlinePhase.value = 'finished'
    onlineStatus.value = `Game ended: ${reason}`
    drawOffer.value = 'none'
    takebackOffer.value = 'none'
    claimAt.value = null
    firstMoveBy.value = null
    clock.pause()
    const config = onlineClockConfig.value
    if (onlineId.value && opponentId.value)
      lastGame.value = {
        id: onlineId.value,
        account: onlineAccount.value || activeAccount(),
        opponent: opponentId.value,
        color: onlineColor.value,
        rated: onlineGameRated.value,
        setup: onlineSetup.value,
        minutes: config ? Math.max(1, Math.round(config.initial / 60)) : undefined,
        increment: config ? config.increment : undefined,
        days: CORRESPONDENCE.includes(onlineDaysPerTurn.value ?? 0)
          ? (onlineDaysPerTurn.value as CorrespondenceDays)
          : undefined,
      }
    if (notify && previous === 'playing') {
      const outcome =
        reason === 'aborted'
          ? 'Game aborted'
          : !winner
            ? 'Draw'
            : winner === onlineColor.value
              ? 'You won'
              : 'You lost'
      notifyDesktop('gameEvents', outcome, `Against ${onlineOpponent.value} · ${reason}.`)
    }
  }
  function readOnlineEvent(event: OnlineEvent): void {
    if (event.type === 'gameStart') {
      if (event.game.speed === 'correspondence' && event.game.gameId !== onlineId.value) return
      if (shownGame !== event.game.gameId) resetGame()
      shownGame = event.game.gameId
      onlineId.value = event.game.gameId
      onlineColor.value = event.game.color ?? 'white'
      onlineOpponent.value = event.game.opponent?.username ?? 'Opponent'
      onlineGameRated.value = event.game.rated ?? false
      opponentId.value = event.game.opponent?.id ?? ''
      onlinePhase.value = 'playing'
      onlineStatus.value = 'Game in progress'
      if (event.game.speed !== 'correspondence')
        notifyDesktop(
          'gameEvents',
          'Game started',
          `You are playing ${onlineOpponent.value} as ${onlineColor.value}.`,
        )
      return
    }
    if (event.type === 'gameFinish') {
      if (event.game.gameId !== onlineId.value) return
      finish(event.game.winner, event.game.status?.name ?? 'finished', true)
      return
    }
    if (event.type === 'opponentGone') {
      if ('id' in event && event.id !== onlineId.value) return
      if (onlinePhase.value === 'playing') {
        opponentGone.value = event.gone
        claimAt.value =
          event.gone && event.claimWinInSeconds !== undefined
            ? performance.now() + event.claimWinInSeconds * 1000
            : null
        onlineStatus.value = event.gone ? 'Opponent disconnected…' : 'Game in progress'
      }
      return
    }
    if (event.type === 'chatLine') {
      if ('id' in event && event.id !== onlineId.value) return
      if (!chatEnabled()) return
      chat.value = [
        ...chat.value,
        {
          user: event.username,
          text: event.text,
          room: event.room === 'spectator' ? ('spectator' as const) : ('player' as const),
        },
      ].slice(-300)
      return
    }
    // Challenges carry no game state; treating them as one would wipe the board.
    if (event.type !== 'gameFull' && event.type !== 'gameState') return
    if ('id' in event && onlineId.value && event.id !== onlineId.value) return
    const state = event.type === 'gameFull' ? event.state : event
    const previousCount = onlineMoves.value.length
    if (event.type === 'gameFull') {
      if (shownGame !== event.id) {
        resetGame()
        shownGame = event.id
        void loadChat(event.id)
      }
      onlineId.value = event.id
      const clockConfig = event.clock
      onlineClockConfig.value =
        clockConfig && clockConfig.initial !== undefined
          ? { initial: clockConfig.initial / 1000, increment: (clockConfig.increment ?? 0) / 1000 }
          : null
      onlineInitial.value = (clockConfig?.initial ?? 0) / 1000
      onlineGameRated.value = event.rated
      onlineSpeed.value = event.speed ?? ''
      onlineDaysPerTurn.value = event.daysPerTurn
      onlineTournament.value = event.tournamentId ?? ''
      const key = (event as { variant?: { key?: string } }).variant?.key
      const variant = variantFromLichess(key)
      onlineUnsupported.value = variant ? '' : (key ?? 'unknown')
      const initial = event.initialFen
      onlineSetup.value = {
        variant: variant ?? 'standard',
        fen: !initial || initial === 'startpos' ? defaultFen(variant ?? 'standard') : initial,
      }
      const ours = (onlineAccount.value || activeAccount()).toLowerCase()
      // Match by account; if neither side does, keep the colour `gameStart` announced.
      if (event.white.id?.toLowerCase() === ours) onlineColor.value = 'white'
      else if (event.black.id?.toLowerCase() === ours) onlineColor.value = 'black'
      const opponent = onlineColor.value === 'white' ? event.black : event.white
      onlineOpponent.value =
        opponent.name ?? (opponent.aiLevel ? `Stockfish level ${opponent.aiLevel}` : 'Opponent')
      onlineOpponentRating.value = opponent.rating
      opponentId.value = opponent.id ?? ''
    }
    onlineMoves.value = String(state.moves ?? '')
      .trim()
      .split(/\s+/)
      .filter(Boolean)
    if (previousCount && onlineMoves.value.length > previousCount) {
      const san = onlineHistory.value.at(-1)
      playMoveSound(san)
      // A move by the other colour than ours is the opponent's.
      if (onlinePosition.value.turn === onlineColor.value) {
        notifyDesktop('opponentMove', `${onlineOpponent.value} moved`, `${san}. Your move.`)
        // A new move answers any takeback request.
        takebackOffer.value = 'none'
      }
    }
    const theirs = onlineColor.value === 'white' ? 'b' : 'w'
    const mine = onlineColor.value === 'white' ? 'w' : 'b'
    const flags = state as {
      wdraw?: boolean
      bdraw?: boolean
      wtakeback?: boolean
      btakeback?: boolean
      expiration?: { idleMillis: number; millisToMove: number }
      winner?: 'white' | 'black'
    }
    const theirDraw = flags[`${theirs}draw`],
      myDraw = flags[`${mine}draw`]
    if (theirDraw && drawOffer.value !== 'theirs' && event.type === 'gameState')
      notifyDesktop('gameEvents', 'Draw offered', `${onlineOpponent.value} offers a draw.`)
    drawOffer.value = theirDraw ? 'theirs' : myDraw ? 'mine' : 'none'
    takebackOffer.value = flags[`${theirs}takeback`]
      ? 'theirs'
      : flags[`${mine}takeback`]
        ? 'mine'
        : 'none'
    firstMoveBy.value = flags.expiration
      ? performance.now() + Math.max(0, flags.expiration.millisToMove - flags.expiration.idleMillis)
      : null
    const running = isGameInProgress(state.status)
    const white = Number(state.wtime ?? 0)
    const black = Number(state.btime ?? 0)
    // Berserking halves the clock; the next state shows it.
    if (onlineTournament.value && onlineClockConfig.value) {
      const half = onlineClockConfig.value.initial * 500
      const mineLeft = onlineColor.value === 'white' ? white : black
      if (ownMoves.value === 0 && mineLeft <= half + 1000 && mineLeft > 0) berserked.value = true
    }
    clock.set({
      white,
      black,
      ticking: running ? onlinePosition.value.turn : undefined,
      initialSeconds: onlineInitial.value || undefined,
    })
    if (running) {
      onlinePhase.value = 'playing'
      if (!opponentGone.value) onlineStatus.value = 'Game in progress'
    } else finish(flags.winner, state.status, true)
  }
  async function onlineMove(uci: string): Promise<void> {
    if (!onlineId.value || onlinePhase.value !== 'playing') return
    const sent = performance.now()
    try {
      await window.kchess.playOnline(onlineId.value, uci)
      moveAckMs.value = Math.round(performance.now() - sent)
    } catch (cause) {
      // The server may have accepted the move before the response was lost. Never replay it.
      console.warn('[online-game] sending move failed:', cause)
      onlinePhase.value = 'disconnected'
      onlineStatus.value =
        'The move could not be confirmed. Reconnect to check the server position.'
      clock.pause()
      fail(cause)
    }
  }
  async function onlineAction(action: OnlineAction): Promise<void> {
    if (!onlineId.value) return
    const done = await run(async () => {
      await window.kchess.onlineAction(onlineId.value, action)
      return true
    })
    if (!done) return
    if (action === 'offerDraw') drawOffer.value = 'mine'
    if (action === 'declineDraw' || action === 'acceptDraw') drawOffer.value = 'none'
    if (action === 'takeback')
      takebackOffer.value = takebackOffer.value === 'theirs' ? 'none' : 'mine'
    if (action === 'declineTakeback') takebackOffer.value = 'none'
    if (action === 'berserk') berserked.value = true
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
