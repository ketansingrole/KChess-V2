import { GameArchive, archiveState } from '@kchess/rules/gameArchive'
import { defineStore } from 'pinia'
import { onScopeDispose, reactive, ref, shallowRef, watch, toRef, type Ref } from 'vue'
import {
  initialLibrary,
  persistSession,
  refreshLibraryLater,
  registerPersistenceFlush,
} from '../utils/library'
import type { ArchivedGame, GameSnapshot, LibrarySnapshot } from '@kchess/rules/library'

export type { ArchivedGame, GameSnapshot }

/**
 * Played games as the core keeps them. Changes show at once and are written shortly after, the
 * latest version of each changed game only; a game the core refuses returns to its saved version.
 */
export const useGameArchiveStore = defineStore('gameArchive', () => {
  const games = shallowRef<ArchivedGame[]>(initialLibrary().games)
  const error = ref('')
  const saved = new Map(games.value.map((game) => [game.id, game]))
  const pending = new Map<string, ArchivedGame>()
  const failed = new Map<string, { game: ArchivedGame; cause: unknown }>()
  const revisions = new Map<string, number>()
  let writing: Promise<void> | undefined
  /** A refresh came while this window's games were still to be saved: read it again after. */
  let deferred = false
  let changes: Promise<void> = Promise.resolve()
  let mutations = 0
  function mutate<T>(work: () => Promise<T>): Promise<T> {
    mutations++
    const result = changes.then(work)
    changes = result.then(
      () => {
        mutations--
      },
      () => {
        mutations--
      },
    )
    return result
  }
  let timer: ReturnType<typeof setTimeout> | undefined
  function revert(id: string): void {
    const previous = saved.get(id)
    games.value = previous
      ? games.value.map((game) => (game.id === id ? previous : game))
      : games.value.filter((game) => game.id !== id)
  }
  async function flush(strict = false): Promise<void> {
    clearTimeout(timer)
    if (strict) {
      for (const { game } of failed.values()) {
        if (!pending.has(game.id)) pending.set(game.id, game)
      }
      failed.clear()
    }
    while (writing || pending.size || mutations) {
      if (!writing && !pending.size) {
        await changes
        continue
      }
      writing ??= writeBatch().finally(() => {
        writing = undefined
      })
      await writing
    }
    if (deferred && !writing && !pending.size && !failed.size) {
      deferred = false
      refreshLibraryLater()
    }
    if (strict && failed.size)
      throw new AggregateError(
        [...failed.values()].map(({ cause }) => cause),
        'Some games could not be saved.',
      )
  }
  async function writeBatch(): Promise<void> {
    const batch = [...pending.values()]
    const versions = new Map(batch.map((game) => [game.id, revisions.get(game.id)]))
    pending.clear()
    for (const game of batch) {
      try {
        const written = await mutate(async () => {
          if (revisions.get(game.id) !== versions.get(game.id)) return false
          await window.kchess.saveArchivedGame(game)
          return true
        })
        if (!written) continue
        saved.set(game.id, game)
        if (revisions.get(game.id) === versions.get(game.id))
          games.value = [game, ...games.value.filter((entry) => entry.id !== game.id)]
        error.value = ''
      } catch (cause) {
        console.warn('[history] saving a game failed:', game.id, cause)
        if (revisions.get(game.id) === versions.get(game.id)) {
          error.value = cause instanceof Error ? cause.message : String(cause)
          failed.set(game.id, { game, cause })
          revert(game.id)
        }
      }
    }
  }
  const flushNow = (): void => {
    void flush().catch((cause) => console.warn('[history] flushing games failed:', cause))
  }
  function save(game: ArchivedGame): void {
    // The rules already answer with a plain snapshot; cloning it here duplicated a JSON
    // stringify/parse of every move on every save.
    const candidate = game
    revisions.set(candidate.id, (revisions.get(candidate.id) ?? 0) + 1)
    failed.delete(candidate.id)
    games.value = [candidate, ...games.value.filter((g) => g.id !== candidate.id)]
    pending.set(candidate.id, candidate)
    clearTimeout(timer)
    if (candidate.finished) flushNow()
    else timer = setTimeout(flushNow, 150)
  }
  async function remove(id: string): Promise<void> {
    const next = games.value.filter((g) => g.id !== id)
    if (next.length === games.value.length) return
    games.value = next
    revisions.set(id, (revisions.get(id) ?? 0) + 1)
    const version = revisions.get(id)
    pending.delete(id)
    failed.delete(id)
    try {
      // Deletion follows any write already sent; batch entries not sent are invalidated above.
      await mutate(() => window.kchess.removeArchivedGame(id))
      saved.delete(id)
    } catch (cause) {
      console.warn('[history] removing a game failed:', id, cause)
      if (revisions.get(id) === version) {
        error.value = cause instanceof Error ? cause.message : String(cause)
        const previous = saved.get(id)
        if (previous) games.value = [previous, ...games.value.filter((game) => game.id !== id)]
      }
    }
  }
  const unregister = registerPersistenceFlush(() => flush(true))
  window.addEventListener('beforeunload', flushNow)
  onScopeDispose(() => {
    flushNow()
    unregister()
    window.removeEventListener('beforeunload', flushNow)
  })
  /** Another KChess window saved or removed a game: show the library's, once this window's are saved. */
  function replaceFromLibrary(snapshot: LibrarySnapshot): void {
    if (writing || pending.size || failed.size) {
      deferred = true
      return
    }
    deferred = false
    games.value = snapshot.games
    saved.clear()
    for (const game of snapshot.games) saved.set(game.id, game)
  }
  return { games, error, save, remove, flush, replaceFromLibrary }
})

/** One stable identity per session: reloads and takebacks update the same history entry. */
export function useGameArchive(
  source: () => GameSnapshot,
  hasPlay: () => boolean,
  resume = true,
): {
  reset: () => void
  save: (finished?: boolean) => void
  error: Ref<string>
} {
  const archive = useGameArchiveStore()
  const kind = source().source
  const host = {
    source,
    hasPlay,
    games: () => archive.games,
    save: (game: ArchivedGame) => archive.save(game),
    remove: (id: string) => {
      void archive.remove(id)
    },
    now: () => Date.now(),
    id: () => crypto.randomUUID(),
  }
  const state = reactive(archiveState(initialLibrary().sessions[`archive:${kind}`], host))
  const session = new GameArchive(state, host, resume)
  const identityPersistence = persistSession(`archive:${kind}`, () => state.identity, undefined, {
    flush: 'sync',
    deep: true,
  })
  void identityPersistence.flush()
  const save = (finished = false): void => session.save(finished)
  const reset = (): void => session.reset()
  watch(source, () => save(), { immediate: true })
  return { save, reset, error: toRef(archive, 'error') }
}
