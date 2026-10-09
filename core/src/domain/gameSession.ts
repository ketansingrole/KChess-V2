import type { ComputerSession, LocalSession, GameSnapshot } from './library'
import type { BestMoveOptions, EngineLevel } from '../contracts/types'
import { engineLevelInfo } from './engineLevels'
import { setupDrawReason, setupPositionAfter } from './chess'
import { boardResult, opponent, timeoutWinner, pgnResult } from './gameResult'
import { isStandardStart, replaySetup, STANDARD_SETUP } from './variant'

export type Color = 'white' | 'black'
export type ComputerClock = NonNullable<ComputerSession['clock']>
export interface ComputerState extends ComputerSession {
  thinking: boolean
  epoch: number
}

export function computerState(saved?: ComputerSession): ComputerState {
  return {
    moves: [...(saved?.moves ?? [])],
    ply: saved?.ply ?? 0,
    level: saved?.level ?? 'club',
    color: saved?.color ?? 'white',
    resigned: saved?.resigned ?? false,
    setup: { ...(saved?.setup ?? STANDARD_SETUP) },
    clock: saved?.clock ? { ...saved.clock } : null,
    times: saved?.times ? { ...saved.times } : null,
    flagged: saved?.flagged ?? null,
    thinking: false,
    epoch: 0,
  }
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

/** Game transitions shared by desktop, CLI and other hosts. State may be reactive. */
export class ComputerGame {
  private turnStarted: number
  private permitted: boolean
  private disposed = false
  private search = Promise.resolve()
  constructor(
    readonly state: ComputerState,
    private host: ComputerHost,
  ) {
    this.turnStarted = host.now()
    this.permitted = host.allowed()
  }
  get position() {
    return setupPositionAfter(this.state.setup, this.state.moves)
  }
  get draw() {
    return setupDrawReason(this.state.setup, this.state.moves)
  }
  get over() {
    return (
      this.state.resigned ||
      Boolean(this.state.flagged) ||
      this.position.isEnd() ||
      Boolean(this.draw)
    )
  }
  get winner(): Color | undefined {
    if (!this.over) return undefined
    if (this.state.resigned) return opponent(this.state.color)
    return this.state.flagged
      ? timeoutWinner(this.position, this.state.flagged)
      : this.position.outcome()?.winner
  }
  get result() {
    if (!this.over) return null
    if (this.state.resigned)
      return { kind: 'loss' as const, title: 'Stockfish won', detail: 'You resigned' }
    const winner = this.winner
    const won = winner === this.state.color
    const detail = this.state.flagged
      ? !winner
        ? 'Time ran out, but mate was impossible'
        : won
          ? 'Stockfish ran out of time'
          : 'You ran out of time'
      : (boardResult(this.position, this.state.setup.variant, this.draw)?.reason ?? 'Game over')
    return {
      kind: !winner ? ('draw' as const) : won ? ('win' as const) : ('loss' as const),
      title: !winner ? 'Draw' : won ? 'You won' : 'Stockfish won',
      detail,
    }
  }
  get running() {
    return this.permitted && Boolean(this.state.times) && !this.over && this.state.moves.length >= 2
  }
  remaining(color: Color, at = this.host.now()): number {
    const left = this.state.times?.[color] ?? 0
    return this.running && this.position.turn === color
      ? Math.max(0, left - (at - this.turnStarted))
      : left
  }
  snapshot(): ComputerSession {
    const { thinking: _thinking, epoch: _epoch, ...state } = this.state
    return {
      ...state,
      moves: [...state.moves],
      setup: { ...state.setup },
      clock: state.clock ? { ...state.clock } : null,
      times: state.times
        ? { white: this.remaining('white'), black: this.remaining('black') }
        : null,
    }
  }
  archiveSnapshot(): GameSnapshot {
    const engine = `Stockfish (${engineLevelInfo(this.state.level).label})`
    return {
      source: 'computer',
      setup: this.state.setup,
      moves: [...this.state.moves],
      white: this.state.color === 'white' ? 'You' : engine,
      black: this.state.color === 'black' ? 'You' : engine,
      result: pgnResult(this.over, this.winner),
      reason: this.result?.detail ?? 'In progress',
      timeControl: this.state.clock
        ? `${this.state.clock.minutes * 60}+${this.state.clock.increment}`
        : '-',
    }
  }
  private cancel(): void {
    this.state.epoch++
    this.state.thinking = false
    void this.host.stopEngine().catch((cause) => {
      console.warn('[computer-game] Could not stop engine:', cause)
      this.host.failed(cause)
    })
  }
  /** Freeze clocks and cancel actual engine work when assistance becomes unavailable. */
  availabilityChanged(): void {
    const permitted = this.host.allowed()
    const at = this.host.now()
    if (this.permitted && !permitted) {
      if (this.state.times)
        this.state.times = {
          white: this.remaining('white', at),
          black: this.remaining('black', at),
        }
      this.cancel()
    }
    if (permitted !== this.permitted) this.turnStarted = at
    this.permitted = permitted
  }
  expire(at = this.host.now()): boolean {
    if (!this.running || this.remaining(this.position.turn, at) > 0) return false
    const turn = this.position.turn
    this.state.times = { ...this.state.times!, [turn]: 0 }
    this.state.flagged = turn
    this.cancel()
    this.host.flagged?.()
    return true
  }
  private commit(uci: string, at: number, computer: boolean): boolean {
    const replayed = replaySetup(this.state.setup, [...this.state.moves, uci])
    if (replayed?.played.length !== this.state.moves.length + 1) return false
    const mover = this.position.turn
    if (this.state.times) {
      const spent = this.running ? at - this.turnStarted : 0
      this.state.times = {
        ...this.state.times,
        [mover]:
          Math.max(0, this.state.times[mover] - spent) +
          (this.state.moves.length >= 2 ? (this.state.clock?.increment ?? 0) * 1000 : 0),
      }
    }
    this.turnStarted = at
    this.state.moves = [...this.state.moves, uci]
    this.state.ply = this.state.moves.length
    this.host.moved?.(replayed.played.at(-1)!.san, computer)
    return true
  }
  move(uci: string): boolean {
    if (
      this.disposed ||
      !this.host.allowed() ||
      !this.host.ready() ||
      this.state.ply !== this.state.moves.length ||
      this.over ||
      this.state.thinking ||
      this.position.turn !== this.state.color
    )
      return false
    const at = this.host.now()
    if (this.expire(at)) return false
    if (!this.commit(uci, at, false)) return false
    void this.computerTurn()
    return true
  }
  computerTurn(): Promise<void> {
    if (this.state.thinking) return this.search
    this.search = this.runComputerTurn()
    return this.search
  }
  private async runComputerTurn(): Promise<void> {
    if (
      this.disposed ||
      this.state.thinking ||
      this.position.turn === this.state.color ||
      this.over ||
      !this.host.ready() ||
      !this.host.allowed()
    )
      return
    const epoch = this.state.epoch,
      count = this.state.moves.length
    this.state.thinking = true
    try {
      const setup = this.state.setup
      const left = this.remaining(this.position.turn)
      const movetime = this.state.times
        ? Math.max(
            50,
            Math.min(
              5000,
              engineLevelInfo(this.state.level).time,
              Math.floor(left / 30 + (this.state.clock?.increment ?? 0) * 800),
            ),
          )
        : undefined
      const move = await this.host.bestMove([...this.state.moves], this.state.level, {
        ...(isStandardStart(setup) ? {} : { fen: setup.fen }),
        ...(setup.variant === 'chess960' ? { chess960: true } : {}),
        ...(movetime ? { movetime } : {}),
      })
      if (
        this.disposed ||
        epoch !== this.state.epoch ||
        count !== this.state.moves.length ||
        this.over ||
        !this.host.allowed()
      )
        return
      const at = this.host.now()
      if (!this.expire(at)) this.commit(move, at, true)
    } catch (cause) {
      if (epoch === this.state.epoch && !this.disposed) this.host.failed(cause)
      else console.debug('[computer-game] Cancelled engine reply:', cause)
    } finally {
      if (epoch === this.state.epoch) this.state.thinking = false
    }
  }
  start(setup = this.state.setup, clock = this.state.clock): void {
    if (this.disposed) return
    this.cancel()
    this.state.setup = setup
    this.state.clock = clock
    this.state.moves = []
    this.state.ply = 0
    this.state.resigned = false
    this.state.flagged = null
    this.state.times = clock ? { white: clock.minutes * 60000, black: clock.minutes * 60000 } : null
    this.turnStarted = this.host.now()
    void this.computerTurn()
  }
  takeback(): void {
    if (this.disposed || !this.state.moves.length) return
    this.cancel()
    this.state.resigned = false
    this.state.flagged = null
    const kept = this.state.moves.slice(0, -1)
    while (kept.length && setupPositionAfter(this.state.setup, kept).turn !== this.state.color)
      kept.pop()
    this.state.moves = kept
    this.state.ply = kept.length
    this.turnStarted = this.host.now()
    void this.computerTurn()
  }
  resign(): void {
    if (this.disposed || !this.state.moves.length || this.over) return
    this.cancel()
    this.state.resigned = true
    this.state.ply = this.state.moves.length
  }
  dispose(): void {
    if (this.disposed) return
    this.disposed = true
    if (this.state.thinking) this.cancel()
    else this.state.epoch++
  }
}

export interface LocalState extends LocalSession {
  paused: boolean
}
export function localState(saved?: LocalSession): LocalState {
  return {
    setup: { ...(saved?.setup ?? STANDARD_SETUP) },
    moves: [...(saved?.moves ?? [])],
    clock: saved?.clock
      ? { white: { ...saved.clock.white }, black: { ...saved.clock.black } }
      : null,
    times: saved?.times ? { ...saved.times } : null,
    result: saved?.result ? { ...saved.result } : null,
    paused: true,
  }
}

export class LocalGame {
  private turnStarted: number
  constructor(
    readonly state: LocalState,
    private now: () => number,
    private flagged: () => void = () => {},
  ) {
    this.turnStarted = now()
  }
  get position() {
    return setupPositionAfter(this.state.setup, this.state.moves)
  }
  get result() {
    return (
      this.state.result ??
      boardResult(
        this.position,
        this.state.setup.variant,
        setupDrawReason(this.state.setup, this.state.moves),
      )
    )
  }
  get over() {
    return Boolean(this.result)
  }
  get running() {
    return (
      Boolean(this.state.times) && !this.over && !this.state.paused && this.state.moves.length > 0
    )
  }
  remaining(color: Color, at = this.now()): number {
    const left = this.state.times?.[color] ?? 0
    return this.running && this.position.turn === color
      ? Math.max(0, left - (at - this.turnStarted))
      : left
  }
  snapshot(): LocalSession {
    const { paused: _paused, ...state } = this.state
    return {
      ...state,
      moves: [...state.moves],
      setup: { ...state.setup },
      clock: state.clock
        ? { white: { ...state.clock.white }, black: { ...state.clock.black } }
        : null,
      result: state.result ? { ...state.result } : null,
      times: state.times
        ? { white: this.remaining('white'), black: this.remaining('black') }
        : null,
    }
  }
  archiveSnapshot(): GameSnapshot {
    const clock = this.state.clock
    return {
      source: 'board',
      setup: this.state.setup,
      moves: [...this.state.moves],
      white: 'White',
      black: 'Black',
      result: pgnResult(this.over, this.result?.winner),
      reason: this.result?.reason ?? 'In progress',
      timeControl: clock
        ? `${clock.white.minutes * 60}+${clock.white.increment} / ${clock.black.minutes * 60}+${clock.black.increment}`
        : '-',
    }
  }
  expire(at = this.now()): boolean {
    if (!this.running || this.remaining(this.position.turn, at) > 0) return false
    const turn = this.position.turn
    this.state.times = { ...this.state.times!, [turn]: 0 }
    const winner = timeoutWinner(this.position, turn)
    this.state.result = winner
      ? { winner, reason: 'Time out' }
      : { reason: 'Time out, but mate was impossible' }
    this.flagged()
    return true
  }
  move(uci: string): string | undefined {
    if (this.over) return undefined
    const at = this.now()
    if (this.expire(at)) return undefined
    const replayed = replaySetup(this.state.setup, [...this.state.moves, uci])
    if (replayed?.played.length !== this.state.moves.length + 1) return undefined
    const mover = this.position.turn
    if (this.state.times)
      this.state.times = {
        ...this.state.times,
        [mover]:
          Math.max(0, this.state.times[mover] - (this.running ? at - this.turnStarted : 0)) +
          (this.state.moves.length ? (this.state.clock?.[mover].increment ?? 0) * 1000 : 0),
      }
    this.turnStarted = at
    this.state.paused = false
    this.state.moves = [...this.state.moves, uci]
    return replayed.played.at(-1)!.san
  }
  takeback(): void {
    if (!this.state.moves.length) return
    this.state.moves = this.state.moves.slice(0, -1)
    this.state.result = null
    this.turnStarted = this.now()
  }
  start(setup = this.state.setup, clock = this.state.clock): void {
    Object.assign(this.state, {
      setup,
      clock,
      moves: [],
      result: null,
      paused: true,
      times: clock
        ? { white: clock.white.minutes * 60000, black: clock.black.minutes * 60000 }
        : null,
    })
    this.turnStarted = this.now()
  }
  resign(color: Color): void {
    if (!this.over)
      this.state.result = {
        winner: opponent(color),
        reason: `${color === 'white' ? 'White' : 'Black'} resigned`,
      }
  }
  agreeDraw(): void {
    if (!this.over) this.state.result = { reason: 'Draw agreed' }
  }
  togglePause(): void {
    const at = this.now()
    if (!this.state.times || this.over || this.expire(at)) return
    if (!this.state.paused)
      this.state.times = { white: this.remaining('white', at), black: this.remaining('black', at) }
    this.state.paused = !this.state.paused
    this.turnStarted = at
  }
}
