import type { ArchiveIdentity, ArchivedGame, GameSnapshot } from './library'
import { assignWritten, transition, type HostRead } from './gameSession.ts'

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

/** What an archive transition tells the driver to do, in order. */
type ArchiveEffect = { type: 'save'; game: ArchivedGame } | { type: 'remove'; id: string }

interface ArchiveDone {
  value: unknown
  state?: ArchiveState
  written: string[]
  effects: ArchiveEffect[]
}

export function archiveState(
  identity: ArchiveIdentity | undefined,
  host: Pick<ArchiveHost, 'now' | 'id'>,
): ArchiveState {
  return transition<{ value: ArchiveState }>('archiveState', [identity ?? null], (kind) => {
    if (kind === 'id') return host.id()
    if (kind === 'now') return host.now()
    throw new Error(`An archive identity has no host read ${kind}`)
  }).value
}

/** Stable archive identity and completion rules shared by every game frontend. */
export class GameArchive {
  constructor(
    readonly state: ArchiveState,
    private host: ArchiveHost,
    resume = true,
  ) {
    this.perform(this.apply('archiveInit', [resume]).effects)
  }
  private read = (kind: HostRead): unknown => {
    const host = this.host
    switch (kind) {
      case 'hasPlay':
        return host.hasPlay()
      case 'source':
        return host.source()
      case 'games':
        return host.games()
      case 'now':
        return host.now()
      case 'id':
        return host.id()
      default:
        throw new Error(`An archive has no host read ${kind}`)
    }
  }
  private apply(method: string, input: unknown[] = []): ArchiveDone {
    const done = transition<ArchiveDone>(method, [this.state, ...input], this.read)
    if (done.state) assignWritten(this.state, done.state, done.written)
    return done
  }
  private perform(effects: readonly ArchiveEffect[]): void {
    for (const effect of effects) {
      if (effect.type === 'save') this.host.save(effect.game)
      else this.host.remove(effect.id)
    }
  }
  save(finished = false): void {
    this.perform(this.apply('archiveSave', [finished]).effects)
  }
  reset(): void {
    this.perform(this.apply('archiveReset').effects)
  }
}
