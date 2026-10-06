import { defineStore } from 'pinia'
import { ref } from 'vue'
import { readSession, persistSession } from '../utils/sessionPersistence'
import { parsePgn, makePgn } from 'chessops/pgn'
import { treeFromPgn } from '../utils/analysisTree'
import type { LichessStudyChapter } from '../../src/shared/types'
export interface StudyChapter {
  id: string
  name: string
  pgn: string
}
export interface SavedStudy {
  id: string
  name: string
  /** First chapter, retained for existing repertoire and preview consumers. */
  pgn: string
  chapters: StudyChapter[]
  updatedAt: number
  cloud?: { account: string; id: string; downloadedPgn: string; structureChanged?: boolean }
}
export const useStudyStore = defineStore('studies', () => {
  const items = ref<SavedStudy[]>(
    readSession('kchess:studies:v1', (raw) => {
      if (
        !raw ||
        typeof raw !== 'object' ||
        !('version' in raw) ||
        ![1, 2].includes(Number(raw.version)) ||
        !('items' in raw) ||
        !Array.isArray(raw.items)
      )
        return undefined
      const result: SavedStudy[] = []
      for (const item of raw.items.slice(0, 50)) {
        if (
          !item ||
          typeof item.id !== 'string' ||
          typeof item.name !== 'string' ||
          typeof item.pgn !== 'string' ||
          !Number.isFinite(item.updatedAt)
        )
          continue
        const chapters = item.chapters ?? [{ id: item.id, name: item.name, pgn: item.pgn }]
        if (
          !Array.isArray(chapters) ||
          !chapters.length ||
          chapters.length > 64 ||
          chapters.some(
            (c) =>
              !c ||
              typeof c.id !== 'string' ||
              typeof c.name !== 'string' ||
              typeof c.pgn !== 'string' ||
              !treeFromPgn(c.pgn),
          )
        )
          continue
        const cloud =
          item.cloud &&
          typeof item.cloud.account === 'string' &&
          typeof item.cloud.id === 'string' &&
          /^[a-zA-Z0-9]{8}$/.test(item.cloud.id) &&
          typeof item.cloud.downloadedPgn === 'string'
            ? item.cloud
            : undefined
        result.push({
          id: item.id,
          name: item.name,
          pgn: chapters[0].pgn,
          chapters,
          updatedAt: item.updatedAt,
          cloud,
        })
      }
      return result
    }) ?? [],
  )
  const error = persistSession('kchess:studies:v1', () => ({ version: 2, items: items.value }))
  function commit(candidate: SavedStudy): string {
    const next = [candidate, ...items.value.filter((item) => item.id !== candidate.id)]
    if (next.length > 50 || JSON.stringify(next).length > 1_800_000)
      throw new Error('The study library is full. Export and remove a study before saving another.')
    items.value = next
    return candidate.id
  }
  function saveChapters(name: string, chapters: LichessStudyChapter[], id?: string): string {
    name = name.trim().slice(0, 120)
    if (
      !name ||
      !chapters.length ||
      chapters.length > 64 ||
      chapters.some((c) => !treeFromPgn(c.pgn))
    )
      throw new Error('Name the study and use up to 64 valid chapters.')
    const old = items.value.find((item) => item.id === id)
    const saved = chapters.map((c, index) => ({
      id: old?.chapters[index]?.id ?? crypto.randomUUID(),
      name: c.name.slice(0, 120),
      pgn: c.pgn,
    }))
    return commit({
      id: id ?? crypto.randomUUID(),
      name,
      pgn: saved[0]!.pgn,
      chapters: saved,
      updatedAt: Date.now(),
      cloud: old?.cloud,
    })
  }
  function save(name: string, pgn: string, id?: string, chapterId?: string): string {
    name = name.trim().slice(0, 120)
    if (!name) throw new Error('Name the study.')
    const old = items.value.find((item) => item.id === id)
    if (!old) return saveChapters(name, [{ name, pgn }], id)
    const index = chapterId ? old.chapters.findIndex((c) => c.id === chapterId) : 0
    if (index < 0 || !treeFromPgn(pgn)) throw new Error('That chapter cannot be saved.')
    const chapters = old.chapters.map((c, i) => (i === index ? { ...c, pgn } : { ...c }))
    return commit({
      ...old,
      name: name.trim().slice(0, 120),
      chapters,
      pgn: chapters[0]!.pgn,
      updatedAt: Date.now(),
    })
  }
  const documentPgn = (study: SavedStudy) =>
    study.chapters
      .map((c) => {
        const game = parsePgn(c.pgn)[0]!
        game.headers.set('ChapterName', c.name)
        return makePgn(game)
      })
      .join('\n\n')
  function content(pgn: string): string {
    return parsePgn(pgn)
      .map((game) => {
        game.headers.delete('Site')
        game.headers.delete('ChapterName')
        return makePgn(game)
      })
      .join('\n\n')
  }
  function markCloud(id: string, account: string, remoteId: string, baseline: string): void {
    const study = items.value.find((s) => s.id === id)
    if (!study) return
    study.cloud = { account, id: remoteId, downloadedPgn: baseline }
    error.flush()
    if (error.value) throw new Error(error.value)
  }
  function offline(
    account: string,
    remote: { id: string; name: string },
    chapters: LichessStudyChapter[],
  ): { id: string; conflict: boolean } {
    const old = items.value.find(
      (s) => s.cloud?.account.toLowerCase() === account.toLowerCase() && s.cloud.id === remote.id,
    )
    const conflict = Boolean(
      old && content(documentPgn(old)) !== content(old.cloud?.downloadedPgn ?? ''),
    )
    const previous = items.value
    // A changed offline copy is retained; cloud updates become a separate copy.
    const id = saveChapters(
      conflict ? `${remote.name} (cloud copy)` : remote.name,
      chapters,
      conflict ? undefined : old?.id,
    )
    const saved = items.value.find((s) => s.id === id)!
    saved.cloud = { account, id: remote.id, downloadedPgn: chapters.map((c) => c.pgn).join('\n\n') }
    error.flush()
    if (error.value) {
      items.value = previous
      throw new Error(error.value)
    }
    return { id, conflict }
  }
  function addChapter(id: string, name: string): string {
    const study = items.value.find((item) => item.id === id)
    if (!study || !name.trim()) throw new Error('Name the new chapter.')
    saveChapters(study.name, [...study.chapters, { name: name.trim(), pgn: '*' }], id)
    return items.value.find((item) => item.id === id)!.chapters.at(-1)!.id
  }
  function duplicateChapter(id: string, chapterId: string): string {
    const study = items.value.find((item) => item.id === id)
    const index = study?.chapters.findIndex((c) => c.id === chapterId) ?? -1
    if (!study || index < 0) throw new Error('That chapter no longer exists.')
    if (study.chapters.length >= 64) throw new Error('A study can contain up to 64 chapters.')
    const original = study.chapters[index]!
    const chapter = {
      ...original,
      id: crypto.randomUUID(),
      name: `${original.name.slice(0, 113)} (copy)`,
    }
    const chapters = [...study.chapters]
    chapters.push(chapter)
    commit({ ...study, chapters, updatedAt: Date.now() })
    return chapter.id
  }
  function removeChapter(id: string, chapterId: string): string {
    const study = items.value.find((item) => item.id === id)
    const index = study?.chapters.findIndex((c) => c.id === chapterId) ?? -1
    if (!study || index < 0) throw new Error('That chapter no longer exists.')
    if (study.chapters.length === 1) throw new Error('A study must keep at least one chapter.')
    const chapters = study.chapters.filter((c) => c.id !== chapterId)
    commit({
      ...study,
      chapters,
      pgn: chapters[0]!.pgn,
      updatedAt: Date.now(),
      cloud: study.cloud ? { ...study.cloud, structureChanged: true } : undefined,
    })
    return chapters[Math.min(index, chapters.length - 1)]!.id
  }
  function renameChapter(id: string, chapterId: string, name: string): void {
    const study = items.value.find((item) => item.id === id)
    if (!study || !name.trim()) return
    const chapters = study.chapters.map((c) =>
      c.id === chapterId ? { ...c, name: name.trim().slice(0, 120) } : c,
    )
    commit({ ...study, chapters, updatedAt: Date.now() })
  }
  function remove(id: string): SavedStudy | undefined {
    const study = items.value.find((item) => item.id === id)
    items.value = items.value.filter((item) => item.id !== id)
    return study
  }
  function restore(study: SavedStudy): void {
    if (items.value.some((item) => item.id === study.id)) return
    commit(study)
  }
  function rename(id: string, name: string): void {
    const study = items.value.find((item) => item.id === id)
    if (study && name.trim())
      commit({ ...study, name: name.trim().slice(0, 120), updatedAt: Date.now() })
  }
  function duplicate(id: string): string | undefined {
    const study = items.value.find((item) => item.id === id)
    return study ? saveChapters(`${study.name.slice(0, 113)} (copy)`, study.chapters) : undefined
  }
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
    saveChapters,
    addChapter,
    renameChapter,
    duplicateChapter,
    removeChapter,
    documentPgn,
    markCloud,
    offline,
    remove,
    restore,
    rename,
    duplicate,
    freshName,
  }
})
