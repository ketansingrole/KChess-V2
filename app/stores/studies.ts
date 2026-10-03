import { defineStore } from 'pinia'
import { ref } from 'vue'
import { readSession, persistSession } from '../utils/sessionPersistence'
import { treeFromPgn } from '../utils/analysisTree'
export interface SavedStudy {
  id: string
  name: string
  pgn: string
  updatedAt: number
}
export const useStudyStore = defineStore('studies', () => {
  const items = ref<SavedStudy[]>(
    readSession('kchess:studies:v1', (raw) => {
      if (
        !raw ||
        typeof raw !== 'object' ||
        !('version' in raw) ||
        raw.version !== 1 ||
        !('items' in raw) ||
        !Array.isArray(raw.items)
      )
        return undefined
      return raw.items
        .slice(0, 50)
        .filter((item): item is SavedStudy =>
          Boolean(
            item &&
            typeof item.id === 'string' &&
            typeof item.name === 'string' &&
            typeof item.pgn === 'string' &&
            Number.isFinite(item.updatedAt) &&
            treeFromPgn(item.pgn),
          ),
        )
    }) ?? [],
  )
  const error = persistSession('kchess:studies:v1', () => ({ version: 1, items: items.value }))
  function save(name: string, pgn: string, id?: string): string {
    name = name.trim().slice(0, 120)
    if (!name || !treeFromPgn(pgn)) throw new Error('Name the study and use a valid game.')
    const candidate = { id: id ?? crypto.randomUUID(), name, pgn, updatedAt: Date.now() }
    const next = [candidate, ...items.value.filter((item) => item.id !== candidate.id)]
    if (next.length > 50 || JSON.stringify(next).length > 1_800_000)
      throw new Error('The study library is full. Export and remove a study before saving another.')
    items.value = next
    return candidate.id
  }
  function remove(id: string): void {
    items.value = items.value.filter((item) => item.id !== id)
  }
  return { items, error, save, remove }
})
