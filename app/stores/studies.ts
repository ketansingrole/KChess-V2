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
  function remove(id: string): SavedStudy | undefined {
    const study = items.value.find((item) => item.id === id)
    items.value = items.value.filter((item) => item.id !== id)
    return study
  }
  /** Put back a study that was just removed (Undo), where it was in the list. */
  function restore(study: SavedStudy): void {
    if (items.value.some((item) => item.id === study.id) || items.value.length >= 50) return
    items.value = [...items.value, study].sort((a, b) => b.updatedAt - a.updatedAt)
  }
  function rename(id: string, name: string): void {
    const study = items.value.find((item) => item.id === id)
    if (study && name.trim() && name.trim() !== study.name) save(name, study.pgn, id)
  }
  function duplicate(id: string): string | undefined {
    const study = items.value.find((item) => item.id === id)
    return study ? save(`${study.name.slice(0, 113)} (copy)`, study.pgn) : undefined
  }
  /** A name no saved study has yet: “Untitled study”, “Untitled study 2” … */
  function freshName(base = 'Untitled study'): string {
    const taken = new Set(items.value.map((item) => item.name))
    for (let n = 1; ; n++) {
      const name = n === 1 ? base : `${base} ${n}`
      if (!taken.has(name)) return name
    }
  }
  return {
    items,
    error,
    flush: () => error.flush(),
    save,
    remove,
    restore,
    rename,
    duplicate,
    freshName,
  }
})
