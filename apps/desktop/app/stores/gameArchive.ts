import { GameArchive, archiveState } from '../../../../core/src/domain/gameArchive'
import { defineStore } from 'pinia'
import { onScopeDispose, reactive, ref, shallowRef, watch, toRef, type Ref } from 'vue'
import { initialLibrary, persistSession, plain, registerPersistenceFlush } from '../utils/library'
import type { ArchivedGame, GameSnapshot } from '../../../../core/src/domain/library'

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
  let timer: ReturnType<typeof setTimeout> | undefined
  function revert(id: string): void {
    const previous = saved.get(id)
    games.value = previous
      ? games.value.map((game) => (game.id === id ? previous : game))
      : games.value.filter((game) => game.id !== id)
  }
  async function flush(): Promise<void> {
    clearTimeout(timer)
    const batch = [...pending.values()]
    pending.clear()
    for (const game of batch) {
      try {
        await window.kchess.saveArchivedGame(game)
        saved.set(game.id, game)
        error.value = ''
      } catch (cause) {
        console.warn('[history] saving a game failed:', game.id, cause)
        error.value = cause instanceof Error ? cause.message : String(cause)
        if (!pending.has(game.id)) revert(game.id)
      }
    }
  }
  const flushNow = (): void => void flush()
  function save(game: ArchivedGame): void {
    const candidate = plain(game)
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
    pending.delete(id)
    try {
      await window.kchess.removeArchivedGame(id)
      saved.delete(id)
    } catch (cause) {
      console.warn('[history] removing a game failed:', id, cause)
      error.value = cause instanceof Error ? cause.message : String(cause)
      revert(id)
    }
  }
  const unregister = registerPersistenceFlush(flush)
  window.addEventListener('beforeunload', flushNow)
  onScopeDispose(() => {
    flushNow()
    unregister()
    window.removeEventListener('beforeunload', flushNow)
  })
  return { games, error, save, remove, flush }
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
  })
  void identityPersistence.flush()
  const save = (finished = false): void => session.save(finished)
  const reset = (): void => session.reset()
  watch(source, () => save(), { deep: true, immediate: true })
  return { save, reset, error: toRef(archive, 'error') }
}
