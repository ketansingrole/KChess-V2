import type { ArchiveIdentity, ArchivedGame, GameSnapshot } from './library'

export interface ArchiveState {
  identity: ArchiveIdentity
  hadPlay: boolean
}
export interface ArchiveHost {
  source(): GameSnapshot
  hasPlay(): boolean
  games(): readonly ArchivedGame[]
  save(game: ArchivedGame): void
  remove(id: string): void
  now(): number
  id(): string
}
export function archiveState(
  identity: ArchiveIdentity | undefined,
  host: Pick<ArchiveHost, 'now' | 'id'>,
): ArchiveState {
  return {
    identity: identity ? { ...identity } : { id: host.id(), startedAt: host.now() },
    hadPlay: false,
  }
}
/** Stable archive identity and completion rules shared by every game frontend. */
export class GameArchive {
  constructor(
    readonly state: ArchiveState,
    private host: ArchiveHost,
    resume = true,
  ) {
    if (!resume) {
      const previous = host.games().find((game) => game.id === state.identity.id)
      if (previous && !previous.finished) host.save({ ...previous, finished: true })
      this.newIdentity()
    }
  }
  private newIdentity(): void {
    this.state.identity = { id: this.host.id(), startedAt: this.host.now() }
  }
  save(finished = false): void {
    if (!this.host.hasPlay()) {
      if (this.state.hadPlay) this.host.remove(this.state.identity.id)
      this.state.hadPlay = false
      return
    }
    this.state.hadPlay = true
    const snapshot = this.host.source()
    this.host.save({
      ...snapshot,
      ...this.state.identity,
      reason:
        finished && snapshot.result === '*' && snapshot.source !== 'clock'
          ? 'Stopped before the game ended'
          : snapshot.reason,
      updatedAt: this.host.now(),
      finished: finished || snapshot.result !== '*',
    })
  }
  reset(): void {
    const previous = this.host.games().find((game) => game.id === this.state.identity.id)
    const latest = this.host.source()
    if (previous && JSON.stringify(previous.moves) === JSON.stringify(latest.moves)) {
      const result = previous.result === '*' ? latest.result : previous.result
      this.host.save({
        ...previous,
        result,
        finished: true,
        reason:
          result === '*' && latest.source !== 'clock'
            ? 'Stopped before the game ended'
            : previous.result === '*'
              ? latest.reason
              : previous.reason,
      })
    } else this.save(true)
    this.state.hadPlay = false
    this.newIdentity()
  }
}
