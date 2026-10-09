import type { CoreApi, NeedsReconnect, Puzzle, PuzzleDifficulty } from '../contracts/types'
import { rules } from './engine.ts'

/**
 * The training session's transitions are Rust (`crates/kchess-domain/src/trainer/puzzle_session.rs`).
 * This class keeps the observable state, runs the host calls each transition asks for, and feeds
 * their replies back with the token it was given. It makes no decisions of its own.
 */
export type TrainMode = 'rated' | 'practice' | 'offline'
export type TrainPhase = 'idle' | 'loading' | 'ready' | 'reconnect' | 'noaccount' | 'nodb' | 'error'

export const isReconnect = (value: unknown): value is NeedsReconnect =>
  rules<boolean>('isReconnect', value)

export interface PuzzleSelection {
  account: string
  angle: string
  difficulty: PuzzleDifficulty
  color: 'random' | 'white' | 'black'
  mode: TrainMode
}

export interface PuzzleSessionState {
  phase: TrainPhase
  current: Puzzle | null
  puzzleKey: number
  isRetry: boolean
  trainError: string
  rating: number | undefined
  session: { solved: number; failed: number; ratingChange: number }
  lastReport: { win: boolean; ratingDiff?: number; failed?: string } | null
  attemptOutcome: boolean | null
}

export function puzzleSessionState(): PuzzleSessionState {
  const { state } = rules<{ state: Omit<PuzzleSessionState, 'rating'> }>('puzzleSessionInitial')
  // JSON has no undefined, so `rating` is absent until a rating arrives. The key must still exist:
  // hosts destructure the state's keys (the store's toRefs), which only sees own properties.
  return { ...state, rating: undefined }
}

/** What the session keeps privately; replies are matched against it (see the Rust module). */
interface Internal {
  loadGen: number
  sessionVersion: number
  attempt: { key: number; account: string; angle: string; mode: TrainMode } | null
}

type CallMethod = 'localPuzzles' | 'puzzleNext' | 'puzzleSolve'
type SettleMethod = 'puzzleSessionLoadSettled' | 'puzzleSessionReportSettled'
type Reply = { ok: unknown } | { error: string }

/** One transition's result: the new state and private state, a host call to await, a warning. */
interface Step {
  state: PuzzleSessionState
  internal: Internal
  call: {
    method: CallMethod
    args: Record<string, unknown>
    resume: Record<string, unknown>
  } | null
  warn: boolean
}

export interface PuzzleSessionHost {
  api: Pick<CoreApi, 'localPuzzles' | 'puzzleNext' | 'puzzleSolve'>
  selection(): PuzzleSelection
  online(): boolean
}

const message = (cause: unknown): string => (cause instanceof Error ? cause.message : String(cause))

/** Attempt identity and reporting belong to a session, independent of board mounts. */
export class PuzzleSession {
  private internal: Internal = rules<{ internal: Internal }>('puzzleSessionInitial').internal

  constructor(
    readonly state: PuzzleSessionState,
    private host: PuzzleSessionHost,
  ) {}

  invalidate(): void {
    this.apply('puzzleSessionInvalidate', {})
  }

  get syncsToLichess(): boolean {
    return rules<boolean>('puzzleSessionSyncs', {
      state: this.state,
      selection: this.host.selection(),
    })
  }

  async loadNext(): Promise<void> {
    const selection = { ...this.host.selection() }
    const step = this.apply('puzzleSessionLoadBegin', {
      selection,
      online: this.host.online(),
    })
    await this.follow(step, 'puzzleSessionLoadSettled')
  }

  show(puzzle: Puzzle, selection: PuzzleSelection = this.host.selection()): void {
    this.apply('puzzleSessionShow', { puzzle, selection: { ...selection } })
  }

  retry(puzzle: Puzzle): void {
    this.apply('puzzleSessionRetry', { puzzle, selection: { ...this.host.selection() } })
  }

  async report(win: boolean): Promise<void> {
    const step = this.apply('puzzleSessionReportBegin', {
      win,
      selection: { ...this.host.selection() },
    })
    await this.follow(step, 'puzzleSessionReportSettled')
  }

  resetSession(): void {
    this.apply('puzzleSessionResetSession', {})
  }

  /** Run the transitions' host calls in order until one asks for nothing more. */
  private async follow(first: Step, settle: SettleMethod): Promise<void> {
    let step = first
    let cause: unknown
    for (;;) {
      if (step.warn)
        console.warn('[puzzleSession] request failed:', cause ?? new Error(step.state.trainError))
      const call = step.call
      if (!call) return
      cause = undefined
      // A rejected request is not swallowed: its message is the reply, which the transition
      // records (as the error or the last report) and asks to be logged when it still counts.
      const reply = await new Promise<unknown>((resolve) =>
        resolve(this.request(call.method, call.args)),
      ).then(
        (ok): Reply => ({ ok }),
        (error: unknown): Reply => {
          cause = error
          return { error: message(error) }
        },
      )
      step = this.apply(settle, {
        resume: call.resume,
        outcome: reply,
        selection: this.host.selection(),
      })
    }
  }

  private request(method: CallMethod, args: Record<string, unknown>): Promise<unknown> {
    const api = this.host.api
    if (method === 'localPuzzles') return api.localPuzzles(args as never)
    if (method === 'puzzleNext') return api.puzzleNext(args as never)
    return api.puzzleSolve(args as never)
  }

  /** Run one transition in Rust and apply its state, in place, to the reactive state object. */
  private apply(method: string, input: Record<string, unknown>): Step {
    const step = rules<Step>(method, {
      state: this.state,
      internal: this.internal,
      ...input,
    })
    const state = this.state
    const next = step.state
    state.phase = next.phase
    // Rust returns a copy: keep the puzzle object hosts already hold unless the puzzle changed,
    // since boards reset when their puzzle object is replaced.
    if (JSON.stringify(state.current) !== JSON.stringify(next.current)) state.current = next.current
    state.puzzleKey = next.puzzleKey
    state.isRetry = next.isRetry
    state.trainError = next.trainError
    state.rating = next.rating
    Object.assign(state.session, next.session)
    state.lastReport = next.lastReport
    state.attemptOutcome = next.attemptOutcome
    this.internal = step.internal
    return step
  }
}
