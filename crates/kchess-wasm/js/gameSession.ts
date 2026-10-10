import type { ComputerSession, LocalSession, GameSnapshot } from './library'
import type { BestMoveOptions, EngineLevel } from '@kchess/contracts/types'
import type { DrawReason } from './chess'
import type { GameResult } from './gameResult'
import { setupPositionAfter } from './chess'
import type { GameSetup } from './variant'
import { rules } from './engine.ts'

export type Color = 'white' | 'black'
export type ComputerClock = NonNullable<ComputerSession['clock']>
export interface ComputerState extends ComputerSession {
  thinking: boolean
  epoch: number
}

/** A host value a transition asks for. Each is read once, when the rules reach it. */
export type HostRead = 'now' | 'allowed' | 'ready' | 'hasPlay' | 'source' | 'games' | 'id'

/**
 * Run a session transition in Rust (`crates/kchess-domain/src/games.rs`). The rules answer with
 * the finished transition or with the next host read they need; the host's getter is called
 * for that read and the rules are asked again, so every read happens once, in the order the
 * rules take it.
 */
export function transition<T>(
  method: string,
  args: unknown[],
  read: (kind: HostRead) => unknown,
): T {
  const reads: Partial<Record<HostRead, unknown[]>> = {}
  for (;;) {
    const out = rules<{ done: T } | { read: HostRead }>(method, ...args, reads)
    if ('done' in out) return out.done
    const values = (reads[out.read] ??= [])
    values.push(read(out.read))
  }
}

/**
 * Assign the fields a transition wrote, as the session always did: a written field is replaced
 * even when its value is unchanged (readers see a new object), and `keep` supplies the caller's
 * own objects for the fields it passed in.
 */
export function assignWritten<T extends object>(
  target: T,
  next: T,
  written: readonly string[],
  keep: Record<string, unknown> = {},
): void {
  const out = target as Record<string, unknown>
  const source = next as Record<string, unknown>
  for (const key of written) out[key] = key in keep ? keep[key] : source[key]
}

export function computerState(saved?: ComputerSession): ComputerState {
  return rules<ComputerState>('computerState', saved ?? null)
}

export interface ComputerHost {
  bestMove(moves: string[], level: EngineLevel, options: BestMoveOptions): Promise<string>
  stopEngine(): Promise<void>
  ready(): boolean
  allowed(): boolean
  now(): number
  moved?(san: string, computer: boolean): void
  flagged?(): void
  failed(cause: unknown): void
}

/** What a computer game's transitions tell the driver to do, in order. */
type ComputerEffect =
  | { type: 'stopEngine' }
  | { type: 'flagged' }
  | { type: 'moved'; san: string; computer: boolean }
  | { type: 'computerTurn' }
  | { type: 'failed' }
  | { type: 'debug' }

/** The private fields a computer game keeps beside its state. */
interface ComputerRuntime {
  turnStarted: number
  permitted: boolean
  disposed: boolean
}

interface ComputerDone<T = unknown> {
  value: T
  state?: ComputerState
  runtime?: ComputerRuntime
  written: string[]
  effects: ComputerEffect[]
}

/** An engine turn to send: its epoch and move count are what the reply must still match. */
interface SearchRequest {
  epoch: number
  count: number
  moves: string[]
  level: EngineLevel
  options: BestMoveOptions
}

export interface ComputerResult {
  kind: 'win' | 'loss' | 'draw'
  title: string
  detail: string
}

/** Game transitions shared by desktop, CLI and other hosts. State may be reactive. */
export class ComputerGame {
  private runtime: ComputerRuntime
  private search = Promise.resolve()
  constructor(
    readonly state: ComputerState,
    private host: ComputerHost,
  ) {
    this.runtime = transition<ComputerDone>('computerInit', [], this.read).runtime!
  }
  private read = (kind: HostRead): unknown => {
    if (kind === 'now') return this.host.now()
    if (kind === 'allowed') return this.host.allowed()
    if (kind === 'ready') return this.host.ready()
    throw new Error(`A computer game has no host read ${kind}`)
  }
  /** Run a transition on the state, keep what it wrote and return what it reports. */
  private apply(
    method: string,
    input: unknown[] = [],
    keep: Record<string, unknown> = {},
  ): ComputerDone {
    const done = transition<ComputerDone>(method, [this.state, this.runtime, ...input], this.read)
    if (done.state) assignWritten(this.state, done.state, done.written, keep)
    if (done.runtime) this.runtime = done.runtime
    return done
  }
  /** Ask a getter of the game; nothing changes. */
  private view<T>(query: string, extra: Record<string, unknown> = {}): T {
    return transition<{ value: T }>(
      'computerView',
      [this.state, this.runtime, { view: query, ...extra }],
      this.read,
    ).value
  }
  /** Carry out what a transition asks of the host, in order. */
  private perform(effects: readonly ComputerEffect[], cause?: unknown): void {
    for (const effect of effects) {
      switch (effect.type) {
        case 'stopEngine':
          void this.host.stopEngine().catch((stopCause) => {
            console.warn('[computer-game] Could not stop engine:', stopCause)
            this.host.failed(stopCause)
          })
          break
        case 'flagged':
          this.host.flagged?.()
          break
        case 'moved':
          this.host.moved?.(effect.san, effect.computer)
          break
        case 'computerTurn':
          void this.computerTurn()
          break
        case 'failed':
          this.host.failed(cause)
          break
        case 'debug':
          console.debug('[computer-game] Cancelled engine reply:', cause)
          break
      }
    }
  }
  get position() {
    return setupPositionAfter(this.state.setup, this.state.moves)
  }
  get draw(): DrawReason | undefined {
    return this.view<DrawReason | null>('draw') ?? undefined
  }
  get over(): boolean {
    return this.view<boolean>('over')
  }
  get winner(): Color | undefined {
    return this.view<Color | null>('winner') ?? undefined
  }
  get result(): ComputerResult | null {
    return this.view<ComputerResult | null>('result')
  }
  get running(): boolean {
    return this.view<boolean>('running')
  }
  remaining(color: Color, at?: number): number {
    return this.view<number>('remaining', { color, at })
  }
  snapshot(): ComputerSession {
    return this.view<ComputerSession>('snapshot')
  }
  archiveSnapshot(): GameSnapshot {
    return this.view<GameSnapshot>('archiveSnapshot')
  }
  /** Freeze clocks and cancel actual engine work when assistance becomes unavailable. */
  availabilityChanged(): void {
    this.perform(this.apply('computerAvailability').effects)
  }
  expire(at?: number): boolean {
    const done = this.apply('computerExpire', [at ?? null])
    this.perform(done.effects)
    return done.value as boolean
  }
  move(uci: string): boolean {
    const done = this.apply('computerMove', [uci])
    this.perform(done.effects)
    return done.value as boolean
  }
  computerTurn(): Promise<void> {
    if (this.state.thinking) return this.search
    this.search = this.runComputerTurn()
    return this.search
  }
  private async runComputerTurn(): Promise<void> {
    const request = this.apply('computerSearchStart').value as SearchRequest | null
    if (!request) return
    const { epoch, count } = request
    try {
      const move = await this.host.bestMove(request.moves, request.level, request.options)
      this.perform(this.apply('computerSearchReply', [{ epoch, count, move }]).effects)
      // eslint-disable-next-line logging/no-silent-catch -- the transition decides: the failure goes to host.failed, or a cancelled reply is logged by its debug effect.
    } catch (cause) {
      this.perform(this.apply('computerSearchFailed', [{ epoch }]).effects, cause)
    } finally {
      this.apply('computerSearchEnd', [{ epoch }])
    }
  }
  start(setup = this.state.setup, clock = this.state.clock): void {
    this.perform(this.apply('computerStart', [setup, clock], { setup, clock }).effects)
  }
  takeback(): void {
    this.perform(this.apply('computerTakeback').effects)
  }
  resign(): void {
    this.perform(this.apply('computerResign').effects)
  }
  dispose(): void {
    this.perform(this.apply('computerDispose').effects)
  }
}

export interface LocalState extends LocalSession {
  paused: boolean
}
export function localState(saved?: LocalSession): LocalState {
  return rules<LocalState>('localState', saved ?? null)
}

/** What a board game's transitions tell the driver to do, in order. */
type LocalEffect = { type: 'flagged' }

interface LocalRuntime {
  turnStarted: number
}

interface LocalDone<T = unknown> {
  value: T
  state?: LocalState
  runtime?: LocalRuntime
  written: string[]
  effects: LocalEffect[]
}

export class LocalGame {
  private runtime: LocalRuntime
  constructor(
    readonly state: LocalState,
    private now: () => number,
    private flagged: () => void = () => {},
  ) {
    this.runtime = transition<LocalDone>('localInit', [], this.read).runtime!
  }
  private read = (kind: HostRead): unknown => {
    if (kind === 'now') return this.now()
    throw new Error(`A local game has no host read ${kind}`)
  }
  private apply(
    method: string,
    input: unknown[] = [],
    keep: Record<string, unknown> = {},
  ): LocalDone {
    const done = transition<LocalDone>(method, [this.state, this.runtime, ...input], this.read)
    if (done.state) assignWritten(this.state, done.state, done.written, keep)
    if (done.runtime) this.runtime = done.runtime
    return done
  }
  private view<T>(query: string, extra: Record<string, unknown> = {}): T {
    return transition<{ value: T }>(
      'localView',
      [this.state, this.runtime, { view: query, ...extra }],
      this.read,
    ).value
  }
  private perform(effects: readonly LocalEffect[]): void {
    for (const effect of effects) if (effect.type === 'flagged') this.flagged()
  }
  get position() {
    return setupPositionAfter(this.state.setup, this.state.moves)
  }
  get result(): GameResult | null {
    return this.view<GameResult | null>('result')
  }
  get over(): boolean {
    return this.view<boolean>('over')
  }
  get running(): boolean {
    return this.view<boolean>('running')
  }
  remaining(color: Color, at?: number): number {
    return this.view<number>('remaining', { color, at })
  }
  snapshot(): LocalSession {
    return this.view<LocalSession>('snapshot')
  }
  archiveSnapshot(): GameSnapshot {
    return this.view<GameSnapshot>('archiveSnapshot')
  }
  expire(at?: number): boolean {
    const done = this.apply('localExpire', [at ?? null])
    this.perform(done.effects)
    return done.value as boolean
  }
  move(uci: string): string | undefined {
    const done = this.apply('localMove', [uci])
    this.perform(done.effects)
    return (done.value as string | null) ?? undefined
  }
  takeback(): void {
    this.perform(this.apply('localTakeback').effects)
  }
  start(setup: GameSetup = this.state.setup, clock = this.state.clock): void {
    this.perform(this.apply('localStart', [setup, clock], { setup, clock }).effects)
  }
  resign(color: Color): void {
    this.perform(this.apply('localResign', [color]).effects)
  }
  agreeDraw(): void {
    this.perform(this.apply('localAgreeDraw').effects)
  }
  togglePause(): void {
    this.perform(this.apply('localTogglePause').effects)
  }
}
