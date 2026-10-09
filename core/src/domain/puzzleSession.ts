import { RequestScope } from './requestScope'
import type { CoreApi, NeedsReconnect, Puzzle, PuzzleDifficulty } from '../contracts/types'
export type TrainMode = 'rated' | 'practice' | 'offline'
export type TrainPhase = 'idle' | 'loading' | 'ready' | 'reconnect' | 'noaccount' | 'nodb' | 'error'
export const isReconnect = (value: unknown): value is NeedsReconnect =>
  typeof value === 'object' && value !== null && 'needsReconnect' in value
const OFFLINE_OFFSET: Record<PuzzleDifficulty, number> = {
  easiest: -600,
  easier: -300,
  normal: 0,
  harder: 300,
  hardest: 600,
}
const message = (cause: unknown): string => (cause instanceof Error ? cause.message : String(cause))
export interface PuzzleSelection {
  account: string
  angle: string
  difficulty: PuzzleDifficulty
  color: 'random' | 'white' | 'black'
  mode: TrainMode
}
export function puzzleSessionState() {
  return {
    phase: 'idle' as TrainPhase,
    current: null as Puzzle | null,
    puzzleKey: 0,
    isRetry: false,
    trainError: '',
    rating: undefined as number | undefined,
    session: { solved: 0, failed: 0, ratingChange: 0 },
    lastReport: null as { win: boolean; ratingDiff?: number; failed?: string } | null,
    attemptOutcome: null as boolean | null,
  }
}
export type PuzzleSessionState = ReturnType<typeof puzzleSessionState>
export interface PuzzleSessionHost {
  api: Pick<CoreApi, 'localPuzzles' | 'puzzleNext' | 'puzzleSolve'>
  selection(): PuzzleSelection
  online(): boolean
}
/** Attempt identity and reporting belong to a session, independent of board mounts. */
export class PuzzleSession {
  constructor(
    readonly state: PuzzleSessionState,
    private host: PuzzleSessionHost,
  ) {}
  invalidate(): void {
    this.loads.invalidate()
    this.attempt = null
    this.state.phase = 'idle'
  }
  private attempt: { key: number; account: string; angle: string; mode: TrainMode } | null = null
  private loads = new RequestScope()
  private sessionVersion = 0

  get syncsToLichess(): boolean {
    return (
      this.host.selection().mode === 'rated' &&
      !this.state.isRetry &&
      Boolean(this.host.selection().account)
    )
  }

  async loadNext(): Promise<void> {
    const selection = { ...this.host.selection() }
    const request = this.loads.next()
    const current = (): boolean =>
      request.current() &&
      Object.entries(selection).every(
        ([key, value]) => this.host.selection()[key as keyof PuzzleSelection] === value,
      )
    this.state.phase = 'loading'
    this.state.trainError = ''
    this.state.lastReport = null
    this.state.isRetry = false
    try {
      if (selection.mode === 'offline') {
        const centre = (this.state.rating ?? 1500) + OFFLINE_OFFSET[selection.difficulty]
        const [puzzle] = await this.host.api.localPuzzles({
          theme: selection.angle,
          minRating: Math.max(0, centre - 150),
          maxRating: centre + 150,
          count: 1,
        })
        if (!current()) return
        if (!puzzle)
          throw new Error('No puzzle of that theme and difficulty in the local database.')
        this.show(puzzle, selection)
        return
      }
      if (selection.mode === 'rated' && !selection.account) {
        this.state.phase = 'noaccount'
        return
      }
      if (!this.host.online())
        throw new Error(
          'You’re offline. Choose Downloaded to use puzzles on this device, or reconnect for online puzzles.',
        )
      const draw = await this.host.api.puzzleNext({
        account: selection.account,
        angle: selection.angle,
        difficulty: selection.difficulty,
        color: selection.color === 'random' ? undefined : selection.color,
      })
      if (!current()) return
      if (isReconnect(draw)) {
        if (selection.mode === 'rated') {
          this.state.phase = 'reconnect'
          return
        }
        // Practice does not need the permission: take a puzzle without the account.
        const anonymous = await this.host.api.puzzleNext({
          account: '',
          angle: selection.angle,
          difficulty: selection.difficulty,
          color: selection.color === 'random' ? undefined : selection.color,
        })
        if (!current()) return
        if (!isReconnect(anonymous)) this.show(anonymous.puzzle, selection)
        return
      }
      if (draw.glicko?.rating !== undefined) this.state.rating = Math.round(draw.glicko.rating)
      this.show(draw.puzzle, selection)
    } catch (cause) {
      if (!current()) return
      console.warn('[puzzleSession] request failed:', cause)
      const text = message(cause)
      if (selection.mode === 'offline' && /Download the puzzle database/.test(text))
        this.state.phase = 'nodb'
      else {
        this.state.trainError = text
        this.state.phase = 'error'
      }
    }
  }

  show(puzzle: Puzzle, selection = this.host.selection()): void {
    this.state.current = puzzle
    this.state.puzzleKey++
    this.state.attemptOutcome = null
    this.attempt = {
      key: this.state.puzzleKey,
      account: selection.account,
      angle: selection.angle,
      mode: selection.mode,
    }
    this.state.phase = 'ready'
  }

  retry(puzzle: Puzzle): void {
    this.loads.invalidate()
    this.state.isRetry = true
    this.state.lastReport = null
    this.show(puzzle)
  }

  async report(win: boolean): Promise<void> {
    const puzzle = this.state.current
    const started = this.attempt
    if (
      !puzzle ||
      !started ||
      this.state.isRetry ||
      this.state.phase !== 'ready' ||
      this.state.attemptOutcome !== null ||
      started.account !== this.host.selection().account ||
      started.mode !== this.host.selection().mode ||
      started.angle !== this.host.selection().angle
    )
      return
    // The verdict belongs to the store's this.attempt, so remounting the board cannot submit it again.
    this.state.attemptOutcome = win
    if (win) this.state.session.solved++
    else this.state.session.failed++
    if (!this.syncsToLichess) return
    const request = this.loads.capture()
    const version = this.sessionVersion
    const stillCurrent = (): boolean =>
      request.current() &&
      version === this.sessionVersion &&
      started === this.attempt &&
      started.account === this.host.selection().account &&
      started.mode === this.host.selection().mode &&
      started.angle === this.host.selection().angle
    try {
      const result = await this.host.api.puzzleSolve({
        account: started.account,
        angle: started.angle,
        id: puzzle.id,
        win,
        rated: true,
      })
      if (!stillCurrent()) return
      if (isReconnect(result)) {
        this.state.phase = 'reconnect'
        this.state.lastReport = {
          win,
          failed: 'Lichess did not accept the result: reconnect your account.',
        }
        return
      }
      if (result.ratingDiff !== undefined) {
        this.state.session.ratingChange += result.ratingDiff
        if (this.state.rating !== undefined) this.state.rating += result.ratingDiff
      }
      if (this.state.current?.id === puzzle.id)
        this.state.lastReport = { win, ratingDiff: result.ratingDiff }
    } catch (cause) {
      if (!stillCurrent()) return
      console.warn('[puzzleSession] request failed:', cause)
      this.state.lastReport = { win, failed: message(cause) }
    }
  }

  resetSession(): void {
    this.sessionVersion++
    this.state.session = { solved: 0, failed: 0, ratingChange: 0 }
  }
}
