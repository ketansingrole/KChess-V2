import { defineStore } from 'pinia'
import { onScopeDispose, ref } from 'vue'
import { initialLibrary, plain, registerPersistenceFlush } from '../utils/library'
import {
  studyDocumentPgn,
  type RepertoireMisses,
  type SavedStudy,
  type StudyChapter,
  type StudyCommand,
  type StudyCommandResult,
} from '@kchess/rules/library'
import type { LichessStudyChapter } from '@kchess/contracts/types'

export type { SavedStudy, StudyChapter }

/** A study's chapters as select items, numbered as the study orders them. */
export const chapterItems = (study: SavedStudy): { label: string; value: string }[] =>
  study.chapters.map((c, index) => ({ label: `${index + 1}. ${c.name}`, value: c.id }))

/** The study library as the core keeps it; every change is made by the core. */
export const useStudyStore = defineStore('studies', () => {
  const items = ref<SavedStudy[]>(initialLibrary().studies)
  const error = ref(initialLibrary().studyError ?? '')
  let changes: Promise<unknown> = Promise.resolve()
  /**
   * A command built when it reaches the core, so values it does not change (a content save's
   * study name) come from the library as earlier commands left it. `undefined`: nothing to do.
   */
  type Intent = () => StudyCommand | undefined
  const failed: { intent: Intent; cause: unknown }[] = []
  const study = (id: string | undefined) => items.value.find((item) => item.id === id)
  /** What a command overwrites; a newer command for the same target makes an older one obsolete. */
  function target(command: StudyCommand): string | undefined {
    switch (command.op) {
      case 'save':
        return command.id
          ? `content ${command.id} ${command.chapterId || study(command.id)?.chapters[0]?.id || ''}`
          : undefined
      case 'rename':
        return `name ${command.id}`
      case 'renameChapter':
        return `chapter name ${command.id} ${command.chapterId}`
      case 'markCloud':
        return `cloud ${command.id}`
      default:
        return undefined
    }
  }
  const supersedes = (newer: StudyCommand, older: StudyCommand): boolean => {
    const key = target(newer)
    return key ? key === target(older) : JSON.stringify(newer) === JSON.stringify(older)
  }
  /** A failed command whose study or chapter has since been removed is not replayed. */
  function applicable(command: StudyCommand): boolean {
    if (command.op === 'restore' || command.op === 'offline' || !command.id) return true
    const open = study(command.id)
    if (!open) return false
    return !('chapterId' in command && command.chapterId)
      ? true
      : open.chapters.some((chapter) => chapter.id === command.chapterId)
  }
  function forget(obsolete: (command: StudyCommand) => boolean): void {
    for (let i = failed.length - 1; i >= 0; i--) {
      const command = failed[i]?.intent()
      if (!command || obsolete(command)) failed.splice(i, 1)
    }
  }
  function run(intent: Intent): Promise<StudyCommandResult> {
    const result = changes.then(() => {
      // A newer command for the same target (or a manual redo) supersedes a failed one, so a
      // later quit retry neither applies the same intent twice nor rolls back newer work. This
      // runs once earlier commands have settled: one still pending when this was queued may
      // only now have failed.
      const queued = intent()
      if (queued) forget((older) => supersedes(queued, older))
      return persist(intent)
    })
    // A failed command must not prevent later commands from reaching the core.
    changes = result.then(
      () => {},
      () => {},
    )
    return result
  }
  const command = (sent: StudyCommand): Promise<StudyCommandResult> => {
    const copy = plain(sent)
    return run(() => copy)
  }
  async function flush(strict = false): Promise<void> {
    let pending: Promise<unknown>
    do {
      pending = changes
      await pending
    } while (pending !== changes)
    if (!strict || !failed.length) return
    // Retry what never reached the core, in order; successes stay cleared,
    // failures re-queue via persist() for the next quit attempt.
    const retry = failed.splice(0, failed.length)
    const causes: unknown[] = []
    for (const { intent } of retry) {
      const sent = intent()
      if (!sent || !applicable(sent)) continue
      try {
        await run(intent)
      } catch (cause) {
        // persist() logged the save itself; note why the quit is vetoed.
        console.warn('[studies] quit flush keeps unsaved study changes:', sent.op, cause)
        causes.push(cause)
      }
    }
    do {
      pending = changes
      await pending
    } while (pending !== changes)
    const remaining = failed.map(({ cause }) => cause)
    if (causes.length || remaining.length)
      throw new AggregateError(
        [...causes, ...remaining.filter((cause) => !causes.includes(cause))],
        'Some studies could not be saved.',
      )
  }
  const unregister = registerPersistenceFlush(() => flush(true))
  onScopeDispose(unregister)
  async function persist(intent: Intent): Promise<StudyCommandResult> {
    const sent = intent()
    if (!sent) return { studies: items.value }
    try {
      const result = await window.kchess.studyCommand(sent, true)
      if (result.order) {
        const byId = new Map(items.value.map((study) => [study.id, study]))
        for (const study of result.studies) byId.set(study.id, study)
        items.value = result.order.map((id) => {
          const study = byId.get(id)
          if (!study) throw new Error('The study library changed. Reopen the app to reload it.')
          return study
        })
      } else items.value = result.studies
      error.value = ''
      // Failed edits to a study or chapter removed since then no longer apply.
      if (sent.op === 'remove') forget((older) => 'id' in older && older.id === sent.id)
      if (sent.op === 'removeChapter')
        forget((older) => 'chapterId' in older && older.chapterId === sent.chapterId)
      return result
    } catch (cause) {
      console.warn('[studies] saving the study library failed:', sent.op, cause)
      error.value = cause instanceof Error ? cause.message : String(cause)
      failed.push({ intent, cause })
      throw cause
    }
  }
  const id = async (sent: StudyCommand): Promise<string> => {
    const result = await command(sent)
    if (!result.id) throw new Error('The study could not be saved.')
    return result.id
  }

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
  /**
   * Save one chapter's moves. The study keeps the name it has when the save reaches the core, so
   * an autosave queued behind a rename does not restore the old name.
   */
  const saveChapter = (studyId: string, chapterId: string, pgn: string) =>
    run(() => {
      const open = study(studyId)
      return open && { op: 'save', name: open.name, pgn, id: studyId, chapterId }
    })
  const addChapter = (studyId: string, name: string) => id({ op: 'addChapter', id: studyId, name })
  const renameChapter = async (studyId: string, chapterId: string, name: string) =>
    void (await command({ op: 'renameChapter', id: studyId, chapterId, name }))
  const duplicateChapter = (studyId: string, chapterId: string) =>
    id({ op: 'duplicateChapter', id: studyId, chapterId })
  const removeChapter = (studyId: string, chapterId: string) =>
    id({ op: 'removeChapter', id: studyId, chapterId })
  const markCloud = async (studyId: string, account: string, remoteId: string, baseline: string) =>
    void (await command({ op: 'markCloud', id: studyId, account, remoteId, baseline }))
  async function offline(
    account: string,
    remote: { id: string; name: string },
    chapters: LichessStudyChapter[],
  ): Promise<{ id: string; conflict: boolean }> {
    const result = await command({
      op: 'offline',
      account,
      remote: { id: remote.id, name: remote.name },
      chapters: chapters.map((c) => ({ name: c.name, pgn: c.pgn })),
    })
    if (!result.id) throw new Error('The study could not be saved offline.')
    return { id: result.id, conflict: result.conflict === true }
  }
  const remove = async (studyId: string): Promise<SavedStudy | undefined> =>
    (await command({ op: 'remove', id: studyId })).removed
  const restore = async (study: SavedStudy) => void (await command({ op: 'restore', study }))
  const rename = async (studyId: string, name: string) =>
    void (await command({ op: 'rename', id: studyId, name }))
  const duplicate = async (studyId: string): Promise<string | undefined> =>
    (await command({ op: 'duplicate', id: studyId })).id
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
    flush,
    error,
    save,
    saveChapter,
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
