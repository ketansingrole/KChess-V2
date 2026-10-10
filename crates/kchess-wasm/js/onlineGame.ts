import type {
  CoreApi,
  ChatLine,
  NotificationKind,
  OnlineAction,
  OnlineEvent,
  OnlineConnection,
  OnlineOptions,
  CorrespondenceDays,
} from '@kchess/contracts/types'
import { STANDARD_SETUP, type GameSetup, type Variant } from './variant'
import { rules } from './engine.ts'
import { Position, type Color } from './position.ts'
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

/** The fields the rules keep beside the public state (`OnlineGame` private and public fields). */
interface Private {
  stateEpoch: number
  generation: number
  shownGame: string
}

/** One effect the host must see, in order (`crates/kchess-domain/src/online_game/model.rs`). */
type Effect =
  | { effect: 'warn'; message: string }
  | { effect: 'notify'; kind: NotificationKind; title: string; body: string }
  | { effect: 'moved'; san: string | null }
  | { effect: 'resetView' }
  | { effect: 'connectionChanged' }
  | { effect: 'moveAcknowledged'; ms: number }
  | { effect: 'failed' }
  | { effect: 'clockPause' }
  | { effect: 'clockSet'; white: number; black: number; ticking?: Color; initialSeconds?: number }
  | { effect: 'loadChat'; id: string }

interface Transition {
  state: OnlineGameState & Private
  effects: Effect[]
  ret?: unknown
  call?: { method: keyof OnlineGameHost['api']; args: unknown[] }
  pending?: unknown
  rethrow: boolean
}

interface View {
  position: { variant: Variant; fen: string }
  history: string[]
  ownMoves: number
  correspondence: boolean
  rematch: Partial<OnlineOptions> | null
}

/** The fields the rules report as `null` where the class keeps `undefined`. */
const UNDEFINED_WHEN_NULL: readonly string[] = ['onlineOpponentRating', 'onlineDaysPerTurn']

/** Equal as the state compares them: same value, or the same JSON for an object. */
function same(a: unknown, b: unknown): boolean {
  if (a === b) return true
  return (
    typeof a === 'object' &&
    typeof b === 'object' &&
    a !== null &&
    b !== null &&
    JSON.stringify(a) === JSON.stringify(b)
  )
}

/** Server events own game state; hosts supply presentation and input. */
export class OnlineGame {
  readonly clock: Clock
  shownGame = ''
  private stateEpoch = -1
  private generation = 0
  constructor(
    readonly state: OnlineGameState,
    private host: OnlineGameHost,
  ) {
    this.clock = new Clock(host.now)
  }
  get position(): Position {
    const { variant, fen } = this.view().position
    return Position.from({ variant, fen })!
  }
  get history(): string[] {
    return this.view().history
  }
  get ownMoves(): number {
    return this.view().ownMoves
  }
  get correspondence(): boolean {
    return this.view().correspondence
  }
  rematchOptions(): Partial<OnlineOptions> | undefined {
    return this.view().rematch ?? undefined
  }
  async start(options: OnlineOptions): Promise<boolean> {
    return (await this.perform('start', options)) as boolean
  }
  async stop(): Promise<void> {
    await this.perform('stop', null)
  }
  async reconnect(): Promise<void> {
    await this.perform('reconnect', null)
  }
  async open(account: string, id: string): Promise<void> {
    await this.perform('open', { account, id })
  }
  resetGame(): void {
    this.transition('onlineGameReset', [])
  }
  readOnlineState(state: OnlineConnection): void {
    this.transition('onlineGameConnection', [state])
  }
  readOnlineEvent(event: OnlineEvent): void {
    this.transition('onlineGameEvent', [event])
  }
  checkingOnline(): void {
    this.transition('onlineGameChecking', [])
  }
  finish(winner: 'white' | 'black' | undefined, reason: string, notify: boolean): void {
    this.transition('onlineGameFinish', [{ winner, reason, notify }])
  }
  async loadChat(id: string): Promise<void> {
    await this.perform('chat', id)
  }
  async onlineMove(uci: string): Promise<void> {
    await this.perform('move', uci)
  }
  async onlineAction(action: OnlineAction): Promise<void> {
    await this.perform('action', action)
  }

  /** Runs a network-backed operation: the rules start it, the host makes the call, the rules settle it. */
  private async perform(operation: string, input: unknown): Promise<unknown> {
    const begun = this.transition('onlineGameBegin', [operation, input])
    if (!begun.call) return begun.ret
    const api = this.host.api as unknown as Record<string, (...args: unknown[]) => Promise<unknown>>
    let value: unknown
    try {
      value = await api[begun.call.method]!(...begun.call.args)
    } catch (cause) {
      const settled = this.transition('onlineGameSettle', [begun.pending, { ok: false }], cause)
      if (settled.rethrow) throw cause
      return settled.ret
    }
    return this.transition('onlineGameSettle', [begun.pending, { ok: true, value }]).ret
  }

  /** Asks the rules for the next state, stores it, then runs its effects in order. */
  private transition(method: string, args: unknown[], cause?: unknown): Transition {
    const next = rules<Transition>(method, this.snapshot(), ...args, this.context())
    this.commit(next, cause)
    return next
  }

  private view(): View {
    return rules<View>('onlineGameView', this.snapshot())
  }

  private snapshot(): OnlineGameState & Private {
    return {
      ...this.state,
      stateEpoch: this.stateEpoch,
      generation: this.generation,
      shownGame: this.shownGame,
    }
  }

  private context(): { now: number; activeAccount: string; chatEnabled: boolean } {
    return {
      now: this.host.now(),
      activeAccount: this.host.activeAccount(),
      chatEnabled: this.host.chatEnabled(),
    }
  }

  private commit(next: Transition, cause: unknown): void {
    const { stateEpoch, generation, shownGame, ...fields } = next.state
    this.stateEpoch = stateEpoch
    this.generation = generation
    this.shownGame = shownGame
    const target = this.state as unknown as Record<string, unknown>
    for (const [key, value] of Object.entries(fields)) {
      const field = value === null && UNDEFINED_WHEN_NULL.includes(key) ? undefined : value
      if (!same(target[key], field)) target[key] = field
    }
    for (const effect of next.effects) this.run(effect, cause)
  }

  private run(effect: Effect, cause: unknown): void {
    switch (effect.effect) {
      case 'warn':
        console.warn(effect.message, cause)
        return
      case 'notify':
        this.host.notify?.(effect.kind, effect.title, effect.body)
        return
      case 'moved':
        this.host.moved?.(effect.san ?? undefined)
        return
      case 'resetView':
        this.host.resetView?.()
        return
      case 'connectionChanged':
        this.host.connectionChanged?.()
        return
      case 'moveAcknowledged':
        this.host.moveAcknowledged?.(effect.ms)
        return
      case 'failed':
        this.host.failed(cause)
        return
      case 'clockPause':
        this.clock.pause()
        return
      case 'clockSet':
        this.clock.set({
          white: effect.white,
          black: effect.black,
          ticking: effect.ticking,
          initialSeconds: effect.initialSeconds,
        })
        return
      case 'loadChat':
        void this.loadChat(effect.id)
        return
    }
  }
}
