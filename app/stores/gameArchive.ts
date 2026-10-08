import { defineStore } from 'pinia'
import { onScopeDispose, ref, shallowRef, watch, toRef, type Ref } from 'vue'
import { initialLibrary, persistSession, plain } from '../utils/library'
import type { ArchiveIdentity, ArchivedGame, GameSnapshot } from '../../src/shared/library'

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
  window.addEventListener('beforeunload', flushNow)
  onScopeDispose(() => {
    flushNow()
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
  const identity = ref<ArchiveIdentity>(
    initialLibrary().sessions[`archive:${kind}`] ?? {
      id: crypto.randomUUID(),
      startedAt: Date.now(),
    },
  )
  persistSession(`archive:${kind}`, () => identity.value, undefined, { flush: 'sync' })
  if (!resume) {
    const previous = archive.games.find((g) => g.id === identity.value.id)
    if (previous && !previous.finished) archive.save({ ...previous, finished: true })
    identity.value = { id: crypto.randomUUID(), startedAt: Date.now() }
  }
  let hadPlay = false
  function save(finished = false): void {
    if (!hasPlay()) {
      if (hadPlay) void archive.remove(identity.value.id)
      hadPlay = false
      return
    }
    hadPlay = true
    const snapshot = source()
    archive.save({
      ...snapshot,
      ...identity.value,
      reason:
        finished && snapshot.result === '*' && kind !== 'clock'
          ? 'Stopped before the game ended'
          : snapshot.reason,
      updatedAt: Date.now(),
      finished: finished || snapshot.result !== '*',
    })
  }
  watch(source, () => save(), { deep: true, immediate: true })
  function reset(): void {
    const previous = archive.games.find((g) => g.id === identity.value.id)
    // New-game forms may already have changed the selected colour or clock.
    // Preserve the last recorded game's metadata when its moves haven't changed.
    if (previous && JSON.stringify(previous.moves) === JSON.stringify(source().moves)) {
      const latest = source()
      const result = previous.result === '*' ? latest.result : previous.result
      archive.save({
        ...previous,
        result,
        finished: true,
        reason:
          result === '*' && kind !== 'clock'
            ? 'Stopped before the game ended'
            : previous.result === '*'
              ? latest.reason
              : previous.reason,
      })
    } else save(true)
    hadPlay = false
    identity.value = { id: crypto.randomUUID(), startedAt: Date.now() }
  }
  return { save, reset, error: toRef(archive, 'error') }
}
