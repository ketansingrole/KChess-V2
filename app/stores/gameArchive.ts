import { defineStore } from 'pinia'
import { ref, watch, toRef, type Ref } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import { readSession, persistSession } from '../utils/sessionPersistence'
import { isVariant, replaySetup, type GameSetup } from '../../src/shared/variant'

export interface ArchivedGame {
  id: string
  source: 'computer' | 'board' | 'clock'
  startedAt: number
  updatedAt: number
  white: string
  black: string
  result: '*' | '1-0' | '0-1' | '1/2-1/2'
  reason: string
  finished: boolean
  setup: GameSetup
  moves: string[]
  timeControl: string
  clockSummary?: string
}
export type GameSnapshot = Omit<ArchivedGame, 'id' | 'startedAt' | 'updatedAt' | 'finished'>

/** Bounded local documents, with semantic validation before a saved game reaches a board. */
export function decodeArchive(raw: unknown): ArchivedGame[] | undefined {
  if (!raw || typeof raw !== 'object') return
  const doc = raw as { version?: unknown; games?: unknown }
  if (doc.version !== 1 || !Array.isArray(doc.games) || doc.games.length > 500) return
  const games: ArchivedGame[] = []
  const ids = new Set<string>()
  for (const rawGame of doc.games) {
    if (!rawGame || typeof rawGame !== 'object') return
    const g = rawGame as ArchivedGame
    if (
      typeof g.id !== 'string' ||
      g.id.length > 80 ||
      ids.has(g.id) ||
      !['computer', 'board', 'clock'].includes(g.source) ||
      !Number.isFinite(g.startedAt) ||
      !Number.isFinite(g.updatedAt) ||
      !['*', '1-0', '0-1', '1/2-1/2'].includes(g.result) ||
      typeof g.finished !== 'boolean' ||
      ![g.white, g.black, g.reason, g.timeControl].every(
        (s) => typeof s === 'string' && s.length <= 200,
      ) ||
      (g.clockSummary !== undefined &&
        (typeof g.clockSummary !== 'string' || g.clockSummary.length > 200)) ||
      !g.setup ||
      !isVariant(g.setup.variant) ||
      typeof g.setup.fen !== 'string' ||
      g.setup.fen.length > 120 ||
      !Array.isArray(g.moves) ||
      g.moves.length > 1024 ||
      !g.moves.every((m) => typeof m === 'string' && m.length <= 10) ||
      replaySetup(g.setup, g.moves)?.played.length !== g.moves.length
    )
      return
    ids.add(g.id)
    games.push(g)
  }
  return games
}

export const useGameArchiveStore = defineStore('gameArchive', () => {
  const games = ref(readSession('kchess:game-history:v1', decodeArchive) ?? [])
  const error = persistSession('kchess:game-history:v1', () => ({ version: 1, games: games.value }))
  function save(game: ArchivedGame): void {
    const next = [game, ...games.value.filter((g) => g.id !== game.id)]
    // Never silently discard older games to make room for a new one.
    if (next.length > 500 || JSON.stringify({ version: 1, games: next }).length > 2_000_000) {
      error.value = 'Game history is full. Export saved games before making room for new games.'
      return
    }
    games.value = next
    error.flush()
  }
  function remove(id: string): void {
    games.value = games.value.filter((g) => g.id !== id)
    error.flush()
  }
  return { games, error, save, remove }
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
  const identity = useLocalStorage(`kchess:history-session:${kind}`, {
    id: crypto.randomUUID(),
    startedAt: Date.now(),
  })
  if (!resume) {
    const previous = archive.games.find((g) => g.id === identity.value.id)
    if (previous && !previous.finished) archive.save({ ...previous, finished: true })
    identity.value = { id: crypto.randomUUID(), startedAt: Date.now() }
  }
  let hadPlay = false
  function save(finished = false): void {
    if (!hasPlay()) {
      if (hadPlay) archive.remove(identity.value.id)
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
