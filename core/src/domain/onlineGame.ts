import type {
  CoreApi,
  ChatLine,
  NotificationKind,
  OnlineAction,
  OnlineEvent,
  OnlineConnection,
  OnlineOptions,
  CorrespondenceDays,
} from '../contracts/types'
import { STANDARD_SETUP, defaultFen, variantFromLichess, type GameSetup } from './variant'
import { setupPositionAfter, setupSanHistory } from './chess'
import { isGameInProgress } from './gameStatus'
import { RequestScope } from './requestScope'
import { Clock } from './clock'

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
export function onlineGameState() {
  return {
    onlinePhase: 'idle' as 'idle' | 'seeking' | 'playing' | 'finished' | 'disconnected',
    onlineId: '',
    onlineAccount: '',
    onlineConnection: null as OnlineConnection | null,
    onlineMoves: [] as string[],
    onlineColor: 'white' as 'white' | 'black',
    onlineOpponent: 'Opponent',
    onlineOpponentRating: undefined as number | undefined,
    onlineStatus: '',
    onlineInitial: 0,
    onlineGameRated: false,
    onlineSetup: STANDARD_SETUP as GameSetup,
    onlineUnsupported: '',
    onlineSpeed: '',
    onlineClockConfig: null as { initial: number; increment: number } | null,
    onlineDaysPerTurn: undefined as number | undefined,
    onlineTournament: '',
    opponentId: '',
    opponentGone: false,
    claimAt: null as number | null,
    drawOffer: 'none' as 'none' | 'mine' | 'theirs',
    takebackOffer: 'none' as 'none' | 'mine' | 'theirs',
    firstMoveBy: null as number | null,
    berserked: false,
    chat: [] as ChatLine[],
    lastGame: null as FinishedGame | null,
  }
}
export type OnlineGameState = ReturnType<typeof onlineGameState>
export interface OnlineGameHost {
  api: Pick<
    CoreApi,
    | 'onlineChat'
    | 'playOnline'
    | 'onlineAction'
    | 'startOnline'
    | 'cancelOnline'
    | 'resumeOnline'
    | 'openGame'
  >
  activeAccount(): string
  chatEnabled(): boolean
  now(): number
  notify?(kind: NotificationKind, title: string, body: string): void
  moved?(san: string | undefined): void
  resetView?(): void
  connectionChanged?(): void
  moveAcknowledged?(milliseconds: number): void
  failed(cause: unknown): void
}
/** Server events own game state; hosts supply presentation and input. */
export class OnlineGame {
  readonly clock: Clock
  shownGame = ''
  private stateEpoch = -1
  private requests = new RequestScope()
  constructor(
    readonly state: OnlineGameState,
    private host: OnlineGameHost,
  ) {
    this.clock = new Clock(host.now)
  }
  get position() {
    return setupPositionAfter(this.state.onlineSetup, this.state.onlineMoves)
  }
  get history() {
    return setupSanHistory(this.state.onlineSetup, this.state.onlineMoves)
  }
  get ownMoves() {
    const whiteFirst = this.state.onlineSetup.fen.split(' ')[1] !== 'b'
    const mine = this.state.onlineColor === 'white' ? 0 : 1
    return this.state.onlineMoves.filter((_, i) => (whiteFirst ? i % 2 : (i + 1) % 2) === mine)
      .length
  }
  get correspondence(): boolean {
    return this.state.onlineSpeed === 'correspondence' || this.state.onlineDaysPerTurn !== undefined
  }
  rematchOptions(): Partial<OnlineOptions> | undefined {
    const game = this.state.lastGame
    if (!game) return undefined
    return {
      target: game.opponent,
      account: game.account,
      color: game.color === 'white' ? 'black' : 'white',
      rated: game.rated,
      ...(game.minutes === undefined ? {} : { minutes: game.minutes }),
      ...(game.increment === undefined ? {} : { increment: game.increment }),
      days: game.days,
      variant: game.setup.variant === 'standard' ? undefined : game.setup.variant,
      fen:
        game.setup.variant === 'standard' && game.setup.fen !== STANDARD_SETUP.fen
          ? game.setup.fen
          : undefined,
    }
  }
  async start(options: OnlineOptions): Promise<boolean> {
    if (
      (this.state.onlinePhase === 'disconnected' && this.state.onlineId) ||
      (this.state.onlinePhase === 'playing' && !this.correspondence) ||
      this.state.onlinePhase === 'seeking'
    )
      return false
    const request = this.requests.next()
    const correspondence = options.days !== undefined
    const previous = this.state.onlinePhase
    if (!correspondence) {
      this.state.onlineId = ''
      this.state.onlineAccount = options.account ?? this.host.activeAccount()
      this.state.onlinePhase = 'seeking'
      this.state.onlineStatus = 'Looking for an opponent…'
    }
    try {
      const result = await this.host.api.startOnline(options)
      if (!request.current()) return false
      if (!correspondence && this.state.onlinePhase === 'seeking')
        this.state.onlineStatus = result.url
          ? 'Challenge sent. Waiting for acceptance…'
          : 'Looking for an opponent…'
      return true
    } catch (cause) {
      console.warn('[onlineGame] starting game failed:', cause)
      if (!request.current()) return false
      if (!correspondence && this.state.onlinePhase === 'seeking') {
        this.state.onlinePhase = previous === 'finished' ? 'finished' : 'idle'
        this.state.onlineStatus = ''
      }
      throw cause
    }
  }
  async stop(): Promise<void> {
    this.requests.invalidate()
    if (this.state.onlinePhase !== 'disconnected' && this.state.onlinePhase !== 'playing') {
      this.state.onlinePhase = 'idle'
      this.state.onlineStatus = ''
    }
    await this.host.api.cancelOnline()
  }
  async reconnect(): Promise<void> {
    const request = this.requests.next()
    this.checkingOnline()
    const resumed = await this.host.api.resumeOnline()
    if (!request.current()) return
    if (resumed) {
      this.state.onlineAccount = resumed.account
      this.state.onlineId = resumed.id
      this.state.onlineStatus = 'Reconnecting…'
    } else {
      this.state.onlinePhase = 'idle'
      this.state.onlineId = ''
      this.state.onlineStatus = 'No game in progress. You can find another game.'
    }
  }
  async open(account: string, id: string): Promise<void> {
    const request = this.requests.next()
    await this.host.api.openGame(account, id)
    if (!request.current()) return
    if (this.shownGame !== id) this.resetGame()
    this.shownGame = ''
    this.state.onlineAccount = account
    this.state.onlineId = id
    this.state.onlineStatus = 'Opening game…'
    if (this.state.onlinePhase !== 'playing') this.state.onlinePhase = 'disconnected'
  }
  resetGame(): void {
    this.host.resetView?.()
    this.state.onlineMoves = []
    this.state.opponentGone = false
    this.state.claimAt = null
    this.state.drawOffer = 'none'
    this.state.takebackOffer = 'none'
    this.state.firstMoveBy = null
    this.state.berserked = false
    this.state.chat = []
    this.state.onlineInitial = 0
    this.state.onlineSetup = STANDARD_SETUP
    this.state.onlineUnsupported = ''
    this.state.onlineSpeed = ''
    this.state.onlineClockConfig = null
    this.state.onlineDaysPerTurn = undefined
    this.state.onlineTournament = ''
    this.state.onlineOpponentRating = undefined
  }
  readOnlineState(state: OnlineConnection): void {
    if (state.session < this.stateEpoch) return
    if (state.session !== this.stateEpoch) {
      this.stateEpoch = state.session
      this.host.connectionChanged?.()
    }
    this.state.onlineConnection = state
    if (state.account) this.state.onlineAccount = state.account
    if (state.gameId) this.state.onlineId = state.gameId
    if (state.lane === 'events' && this.state.onlineId) return
    if (state.lane === 'game' && state.phase === 'idle') {
      this.state.onlinePhase = 'idle'
      this.state.onlineId = ''
      this.state.onlineStatus = 'No game in progress.'
      this.clock.pause()
    }
    if (['checking', 'reconnecting', 'disconnected', 'auth-required'].includes(state.phase)) {
      this.state.onlinePhase = 'disconnected'
      this.state.onlineStatus =
        state.message ?? 'Connection interrupted. Reconnect to recover your game.'
      this.clock.pause()
    }
  }
  checkingOnline(): void {
    this.state.onlinePhase = 'disconnected'
    this.state.onlineStatus = 'Checking Lichess for an ongoing game…'
    this.state.onlineConnection = {
      session: this.stateEpoch,
      account: this.state.onlineAccount,
      gameId: this.state.onlineId,
      lane: 'game',
      phase: 'checking',
      message: this.state.onlineStatus,
    }
    this.clock.pause()
  }
  finish(winner: 'white' | 'black' | undefined, reason: string, notify: boolean): void {
    const previous = this.state.onlinePhase
    this.state.onlinePhase = 'finished'
    this.state.onlineStatus = `Game ended: ${reason}`
    this.state.drawOffer = 'none'
    this.state.takebackOffer = 'none'
    this.state.claimAt = null
    this.state.firstMoveBy = null
    this.clock.pause()
    const config = this.state.onlineClockConfig
    if (this.state.onlineId && this.state.opponentId)
      this.state.lastGame = {
        id: this.state.onlineId,
        account: this.state.onlineAccount || this.host.activeAccount(),
        opponent: this.state.opponentId,
        color: this.state.onlineColor,
        rated: this.state.onlineGameRated,
        setup: this.state.onlineSetup,
        minutes: config ? Math.max(1, Math.round(config.initial / 60)) : undefined,
        increment: config ? config.increment : undefined,
        days: CORRESPONDENCE.includes(this.state.onlineDaysPerTurn ?? 0)
          ? (this.state.onlineDaysPerTurn as CorrespondenceDays)
          : undefined,
      }
    if (notify && previous === 'playing') {
      const outcome =
        reason === 'aborted'
          ? 'Game aborted'
          : !winner
            ? 'Draw'
            : winner === this.state.onlineColor
              ? 'You won'
              : 'You lost'
      this.host.notify?.('gameEvents', outcome, `Against ${this.state.onlineOpponent} · ${reason}.`)
    }
  }
  readOnlineEvent(event: OnlineEvent): void {
    if (event.type === 'gameStart') {
      if (event.game.speed === 'correspondence' && event.game.gameId !== this.state.onlineId) return
      if (this.shownGame !== event.game.gameId) this.resetGame()
      this.shownGame = event.game.gameId
      this.state.onlineId = event.game.gameId
      this.state.onlineColor = event.game.color ?? 'white'
      this.state.onlineOpponent = event.game.opponent?.username ?? 'Opponent'
      this.state.onlineGameRated = event.game.rated ?? false
      this.state.opponentId = event.game.opponent?.id ?? ''
      this.state.onlinePhase = 'playing'
      this.state.onlineStatus = 'Game in progress'
      if (event.game.speed !== 'correspondence')
        this.host.notify?.(
          'gameEvents',
          'Game started',
          `You are playing ${this.state.onlineOpponent} as ${this.state.onlineColor}.`,
        )
      return
    }
    if (event.type === 'gameFinish') {
      if (event.game.gameId !== this.state.onlineId) return
      this.finish(event.game.winner, event.game.status?.name ?? 'finished', true)
      return
    }
    if (event.type === 'opponentGone') {
      if ('id' in event && event.id !== this.state.onlineId) return
      if (this.state.onlinePhase === 'playing') {
        this.state.opponentGone = event.gone
        this.state.claimAt =
          event.gone && event.claimWinInSeconds !== undefined
            ? this.host.now() + event.claimWinInSeconds * 1000
            : null
        this.state.onlineStatus = event.gone ? 'Opponent disconnected…' : 'Game in progress'
      }
      return
    }
    if (event.type === 'chatLine') {
      if ('id' in event && event.id !== this.state.onlineId) return
      if (!this.host.chatEnabled()) return
      this.state.chat = [
        ...this.state.chat,
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
    if ('id' in event && this.state.onlineId && event.id !== this.state.onlineId) return
    const state = event.type === 'gameFull' ? event.state : event
    const previousCount = this.state.onlineMoves.length
    if (event.type === 'gameFull') {
      if (this.shownGame !== event.id) {
        this.resetGame()
        this.shownGame = event.id
        void this.loadChat(event.id)
      }
      this.state.onlineId = event.id
      const clockConfig = event.clock
      this.state.onlineClockConfig =
        clockConfig && clockConfig.initial !== undefined
          ? { initial: clockConfig.initial / 1000, increment: (clockConfig.increment ?? 0) / 1000 }
          : null
      this.state.onlineInitial = (clockConfig?.initial ?? 0) / 1000
      this.state.onlineGameRated = event.rated
      this.state.onlineSpeed = event.speed ?? ''
      this.state.onlineDaysPerTurn = event.daysPerTurn
      this.state.onlineTournament = event.tournamentId ?? ''
      const key = (event as { variant?: { key?: string } }).variant?.key
      const variant = variantFromLichess(key)
      this.state.onlineUnsupported = variant ? '' : (key ?? 'unknown')
      const initial = event.initialFen
      this.state.onlineSetup = {
        variant: variant ?? 'standard',
        fen: !initial || initial === 'startpos' ? defaultFen(variant ?? 'standard') : initial,
      }
      const ours = (this.state.onlineAccount || this.host.activeAccount()).toLowerCase()
      // Match by account; if neither side does, keep the colour `gameStart` announced.
      if (event.white.id?.toLowerCase() === ours) this.state.onlineColor = 'white'
      else if (event.black.id?.toLowerCase() === ours) this.state.onlineColor = 'black'
      const opponent = this.state.onlineColor === 'white' ? event.black : event.white
      this.state.onlineOpponent =
        opponent.name ?? (opponent.aiLevel ? `Stockfish level ${opponent.aiLevel}` : 'Opponent')
      this.state.onlineOpponentRating = opponent.rating
      this.state.opponentId = opponent.id ?? ''
    }
    this.state.onlineMoves = String(state.moves ?? '')
      .trim()
      .split(/\s+/)
      .filter(Boolean)
    if (previousCount && this.state.onlineMoves.length > previousCount) {
      const san = this.history.at(-1)
      this.host.moved?.(san)
      // A move by the other colour than ours is the opponent's.
      if (this.position.turn === this.state.onlineColor) {
        this.host.notify?.(
          'opponentMove',
          `${this.state.onlineOpponent} moved`,
          `${san}. Your move.`,
        )
        // A new move answers any takeback request.
        this.state.takebackOffer = 'none'
      }
    }
    const theirs = this.state.onlineColor === 'white' ? 'b' : 'w'
    const mine = this.state.onlineColor === 'white' ? 'w' : 'b'
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
    if (theirDraw && this.state.drawOffer !== 'theirs' && event.type === 'gameState')
      this.host.notify?.(
        'gameEvents',
        'Draw offered',
        `${this.state.onlineOpponent} offers a draw.`,
      )
    this.state.drawOffer = theirDraw ? 'theirs' : myDraw ? 'mine' : 'none'
    this.state.takebackOffer = flags[`${theirs}takeback`]
      ? 'theirs'
      : flags[`${mine}takeback`]
        ? 'mine'
        : 'none'
    this.state.firstMoveBy = flags.expiration
      ? this.host.now() + Math.max(0, flags.expiration.millisToMove - flags.expiration.idleMillis)
      : null
    const running = isGameInProgress(state.status)
    const white = Number(state.wtime ?? 0)
    const black = Number(state.btime ?? 0)
    // Berserking halves the clock; the next state shows it.
    if (this.state.onlineTournament && this.state.onlineClockConfig) {
      const half = this.state.onlineClockConfig.initial * 500
      const mineLeft = this.state.onlineColor === 'white' ? white : black
      if (this.ownMoves === 0 && mineLeft <= half + 1000 && mineLeft > 0)
        this.state.berserked = true
    }
    this.clock.set({
      white,
      black,
      ticking: running ? this.position.turn : undefined,
      initialSeconds: this.state.onlineInitial || undefined,
    })
    if (running) {
      this.state.onlinePhase = 'playing'
      if (!this.state.opponentGone) this.state.onlineStatus = 'Game in progress'
    } else this.finish(flags.winner, state.status, true)
  }
  async loadChat(id: string): Promise<void> {
    if (!this.host.chatEnabled()) return
    try {
      const lines = await this.host.api.onlineChat(id)
      if (this.state.onlineId === id)
        this.state.chat = [...lines, ...this.state.chat.filter((l) => l.room !== 'player')]
    } catch (cause) {
      console.warn('[onlineGame] request failed:', cause)
      // The chat is a courtesy; the game goes on without it.
    }
  }
  async onlineMove(uci: string): Promise<void> {
    if (!this.state.onlineId || this.state.onlinePhase !== 'playing') return
    const request = this.requests.capture()
    const id = this.state.onlineId
    const epoch = this.stateEpoch
    const current = (): boolean =>
      request.current() && id === this.state.onlineId && epoch === this.stateEpoch
    const sent = this.host.now()
    try {
      await this.host.api.playOnline(id, uci)
      if (!current()) return
      this.host.moveAcknowledged?.(Math.round(this.host.now() - sent))
    } catch (cause) {
      // The server may have accepted the move before the response was lost. Never replay it.
      console.warn('[onlineGame] request failed:', cause)
      if (!current()) return
      this.state.onlinePhase = 'disconnected'
      this.state.onlineStatus =
        'The move could not be confirmed. Reconnect to check the server position.'
      this.clock.pause()
      this.host.failed(cause)
    }
  }
  async onlineAction(action: OnlineAction): Promise<void> {
    if (!this.state.onlineId) return
    const request = this.requests.capture()
    const id = this.state.onlineId
    const epoch = this.stateEpoch
    try {
      await this.host.api.onlineAction(id, action)
      if (!request.current() || id !== this.state.onlineId || epoch !== this.stateEpoch) return
    } catch (cause) {
      console.warn('[onlineGame] request failed:', cause)
      this.host.failed(cause)
      return
    }
    if (action === 'offerDraw') this.state.drawOffer = 'mine'
    if (action === 'declineDraw' || action === 'acceptDraw') this.state.drawOffer = 'none'
    if (action === 'takeback')
      this.state.takebackOffer = this.state.takebackOffer === 'theirs' ? 'none' : 'mine'
    if (action === 'declineTakeback') this.state.takebackOffer = 'none'
    if (action === 'berserk') this.state.berserked = true
  }
}
