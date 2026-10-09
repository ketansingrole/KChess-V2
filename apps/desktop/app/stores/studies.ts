import { defineStore } from 'pinia'
import { ref } from 'vue'
import { initialLibrary, plain } from '../utils/library'
import {
  studyDocumentPgn,
  type RepertoireMisses,
  type SavedStudy,
  type StudyChapter,
  type StudyCommand,
  type StudyCommandResult,
} from '@kchess/core/domain/library'
import type { LichessStudyChapter } from '@kchess/core/contracts/types'

export type { SavedStudy, StudyChapter }

/** The study library as the core keeps it; every change is made by the core. */
export const useStudyStore = defineStore('studies', () => {
  const items = ref<SavedStudy[]>(initialLibrary().studies)
  const error = ref('')
  let latest = 0
  async function run(command: StudyCommand): Promise<StudyCommandResult> {
    const request = ++latest
    try {
      const result = await window.kchess.studyCommand(plain(command))
      // A newer change already carries a newer library.
      if (request === latest) items.value = result.studies
      error.value = ''
      return result
    } catch (cause) {
      console.warn('[studies] saving the study library failed:', command.op, cause)
      error.value = cause instanceof Error ? cause.message : String(cause)
      throw cause
    }
  }
  const id = async (command: StudyCommand): Promise<string> => (await run(command)).id!

  async function saveChapters(
    name: string,
    chapters: LichessStudyChapter[],
    studyId?: string,
  ): Promise<string> {
    return id({
      op: 'saveChapters',
      name,
      chapters: chapters.map((c) => ({ name: c.name, pgn: c.pgn })),
      id: studyId,
    })
  }
  const save = (name: string, pgn: string, studyId?: string, chapterId?: string) =>
    id({ op: 'save', name, pgn, id: studyId, chapterId })
  const addChapter = (studyId: string, name: string) => id({ op: 'addChapter', id: studyId, name })
  const renameChapter = async (studyId: string, chapterId: string, name: string) =>
    void (await run({ op: 'renameChapter', id: studyId, chapterId, name }))
  const duplicateChapter = (studyId: string, chapterId: string) =>
    id({ op: 'duplicateChapter', id: studyId, chapterId })
  const removeChapter = (studyId: string, chapterId: string) =>
    id({ op: 'removeChapter', id: studyId, chapterId })
  const markCloud = async (studyId: string, account: string, remoteId: string, baseline: string) =>
    void (await run({ op: 'markCloud', id: studyId, account, remoteId, baseline }))
  async function offline(
    account: string,
    remote: { id: string; name: string },
    chapters: LichessStudyChapter[],
  ): Promise<{ id: string; conflict: boolean }> {
    const result = await run({
      op: 'offline',
      account,
      remote: { id: remote.id, name: remote.name },
      chapters: chapters.map((c) => ({ name: c.name, pgn: c.pgn })),
    })
    return { id: result.id!, conflict: result.conflict === true }
  }
  const remove = async (studyId: string): Promise<SavedStudy | undefined> =>
    (await run({ op: 'remove', id: studyId })).removed
  const restore = async (study: SavedStudy) => void (await run({ op: 'restore', study }))
  const rename = async (studyId: string, name: string) =>
    void (await run({ op: 'rename', id: studyId, name }))
  const duplicate = async (studyId: string): Promise<string | undefined> =>
    (await run({ op: 'duplicate', id: studyId })).id
  /** Missed repertoire moves by `studyId:color`, then by FEN: the trainer steers towards them. */
  const misses = ref<RepertoireMisses>(initialLibrary().repertoireMisses)
  function recordMiss(key: string, fen: string): void {
    const table = { ...(misses.value[key] ?? {}) }
    table[fen] = (table[fen] ?? 0) + 1
    misses.value = { ...misses.value, [key]: table }
    window.kchess.recordRepertoireMiss(key, fen).then(
      (saved) => (misses.value = saved),
      (cause: unknown) => console.warn('[studies] saving a repertoire miss failed:', cause),
    )
  }
  function clearMisses(key: string): void {
    const next = { ...misses.value }
    delete next[key]
    misses.value = next
    window.kchess.clearRepertoireMisses(key).then(
      (saved) => (misses.value = saved),
      (cause: unknown) => console.warn('[studies] clearing repertoire misses failed:', cause),
    )
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
    save,
    saveChapters,
    addChapter,
    renameChapter,
    duplicateChapter,
    removeChapter,
    documentPgn: studyDocumentPgn,
    markCloud,
    offline,
    remove,
    restore,
    rename,
    duplicate,
    freshName,
    misses,
    recordMiss,
    clearMisses,
  }
})
