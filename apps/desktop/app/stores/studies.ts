import { defineStore } from 'pinia'
import { onScopeDispose, ref } from 'vue'
import {
  initialLibrary,
  libraryWrites,
  plain,
  refreshLibraryLater,
  registerPersistenceFlush,
} from '../utils/library'
import {
  studyDocumentPgn,
  type RepertoireMisses,
  type SavedStudy,
  type StudyChapter,
  type StudyCommand,
  type StudyCommandResult,
  type LibrarySnapshot,
} from '@kchess/rules/library'
import type { LichessStudyChapter } from '@kchess/contracts/types'

export type { SavedStudy, StudyChapter }

/** The core refused a chapter save: another window changed the chapter since this one read it. */
export const isStaleStudy = (cause: unknown): boolean =>
  String(cause instanceof Error ? cause.message : cause).includes(
    'This study was changed in another KChess window.',
  )

/** A study's chapters as select items, numbered as the study orders them. */
export const chapterItems = (study: SavedStudy): { label: string; value: string }[] =>
  study.chapters.map((c, index) => ({ label: `${index + 1}. ${c.name}`, value: c.id }))

/** The study library as the core keeps it; every change is made by the core. */
export const useStudyStore = defineStore('studies', () => {
  const items = ref<SavedStudy[]>(initialLibrary().studies)
  const error = ref(initialLibrary().studyError ?? '')
  let changes: Promise<unknown> = Promise.resolve()
  const writes = libraryWrites()
  /**
   * A command built when it reaches the core, so values it does not change (a content save's
   * study name) come from the library as earlier commands left it. `undefined`: nothing to do.
   */
  type Intent = () => StudyCommand | undefined
  const failed: { intent: Intent; cause: unknown }[] = []
  /** A refresh came while edits were still to be saved: read the library again once they are. */
  let deferred = false
  function settled(): void {
    if (!deferred || failed.length) return
    deferred = false
    refreshLibraryLater()
  }
  const study = (id: string | undefined) => items.value.find((item) => item.id === id)
  /**
   * The moves each chapter held when this window last opened or saved it, by `studyId chapterId`.
   * Chapter saves are based on them (`basedOn`), so they never overwrite another window's newer
   * moves. A library refresh leaves them, since the open board still shows the older moves.
   */
  const bases = new Map<string, string>()
  const baseKey = (studyId: string, chapterId: string) => `${studyId} ${chapterId}`
  /** The board shows this chapter as the library has it: its saves are based on that. */
  function track(studyId: string, chapterId: string): void {
    const pgn = study(studyId)?.chapters.find((chapter) => chapter.id === chapterId)?.pgn
    if (pgn === undefined) bases.delete(baseKey(studyId, chapterId))
    else bases.set(baseKey(studyId, chapterId), pgn)
  }
  /** The chapter a content save writes, once the study exists. */
  function savedChapter(sent: StudyCommand, id = 'id' in sent ? sent.id : undefined) {
    if (sent.op !== 'save' || !id) return undefined
    const chapterId = sent.chapterId || study(id)?.chapters[0]?.id
    return chapterId ? baseKey(id, chapterId) : undefined
  }
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
  /**
   * A command that adds a document each time the core applies it. A failure may only have lost its
   * reply after the core stored it, so it is never sent again: its caller reports the failure.
   */
  const creates = (command: StudyCommand): boolean =>
    command.op === 'duplicate' ||
    command.op === 'addChapter' ||
    command.op === 'duplicateChapter' ||
    ((command.op === 'save' || command.op === 'saveChapters') && !command.id)
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
    void writes.track(result)
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
    // Retry failed commands in order; each one replaces what it targets, so applying it again
    // is harmless. Successes stay cleared, failures re-queue via persist() for the next quit.
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
    const based = savedChapter(sent)
    let result: StudyCommandResult
    try {
      result = await window.kchess.studyCommand(sent, true, based && bases.get(based))
    } catch (cause) {
      console.warn('[studies] saving the study library failed:', sent.op, cause)
      error.value = cause instanceof Error ? cause.message : String(cause)
      // Sent again, it would be refused again: the window reopens the chapter instead.
      if (!isStaleStudy(cause) && !creates(sent)) failed.push({ intent, cause })
      throw cause
    }
    // The core has stored the command: from here nothing is sent again.
    if (result.order) {
      const byId = new Map(items.value.map((study) => [study.id, study]))
      for (const study of result.studies) byId.set(study.id, study)
      // A study another window added that this one has not read yet: read the library for it.
      const known = result.order.flatMap((id) => byId.get(id) ?? [])
      items.value = known
      if (known.length < result.order.length) refreshLibraryLater()
    } else items.value = result.studies
    error.value = ''
    const written = sent.op === 'save' && savedChapter(sent, result.id)
    if (written) bases.set(written, sent.pgn)
    // Failed edits to a study or chapter removed since then no longer apply.
    if (sent.op === 'remove') forget((older) => 'id' in older && older.id === sent.id)
    if (sent.op === 'removeChapter')
      forget((older) => 'chapterId' in older && older.chapterId === sent.chapterId)
    settled()
    return result
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
  /**
   * The core answers each change with the whole table: only the newest change's answer is shown,
   * since an older one, arriving late, predates the changes made after it.
   */
  let missChanges = 0
  function changeMisses(next: RepertoireMisses, work: Promise<RepertoireMisses>, what: string) {
    misses.value = next
    const change = ++missChanges
    writes.track(work).then(
      (saved) => {
        if (change === missChanges) misses.value = saved
      },
      (cause: unknown) => console.warn(`[studies] ${what} failed:`, cause),
    )
  }
  function recordMiss(key: string, fen: string): void {
    const table = { ...(misses.value[key] ?? {}) }
    table[fen] = (table[fen] ?? 0) + 1
    changeMisses(
      { ...misses.value, [key]: table },
      window.kchess.recordRepertoireMiss(key, fen),
      'saving a repertoire miss',
    )
  }
  function clearMisses(key: string): void {
    const next = { ...misses.value }
    delete next[key]
    changeMisses(next, window.kchess.clearRepertoireMisses(key), 'clearing repertoire misses')
  }
  function freshName(base = 'Untitled study'): string {
    const taken = new Set(items.value.map((item) => item.name))
    for (let n = 1; ; n++) {
      const name = n === 1 ? base : `${base} ${n}`
      if (!taken.has(name)) return name
    }
  }
  /**
   * Another KChess window changed the library (`docs/shared-core.md`): show its studies, unless
   * this window still has edits to save, which are saved first and then shown. A snapshot read
   * (at `since`, from `revision()`) before this window's own change finished may predate it.
   */
  function replaceFromLibrary(snapshot: LibrarySnapshot, since = writes.revision()): void {
    if (failed.length) {
      deferred = true
      return
    }
    if (!writes.fresh(since)) return
    deferred = false
    items.value = snapshot.studies
    misses.value = snapshot.repertoireMisses
  }
  return {
    items,
    flush,
    track,
    replaceFromLibrary,
    revision: writes.revision,
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
