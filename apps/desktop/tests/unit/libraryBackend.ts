import { afterAll } from 'vitest'
import { createPinia, disposePinia, getActivePinia, setActivePinia } from 'pinia'
import { flushPromises } from '@vue/test-utils'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { closeNativeCore, nativeCallSync } from '@kchess/native/nativeCore'
import { useTestDatabase, type TestDatabase } from '../../../../tests/fixtures/nativeStore'
import {
  assertArchivedGameShape,
  assertLegacyDocuments,
  assertSession,
  assertSessionKind,
  assertStudyCommandShape,
  type ArchivedGame,
  type JoinedTournament,
  type LegacyDocuments,
  type LibrarySnapshot,
  type MistakeExercise,
  type RepertoireMisses,
  type SessionKind,
  type StudyCommand,
  type StudyCommandResult,
} from '@kchess/rules/library'
import { assertArchivedGame, assertStudyCommand } from '@kchess/native/rules'
import type { CoreApi } from '@kchess/contracts/types'
import { setInitialLibrary } from '../../app/utils/library'
import { testGeneration, type LibraryMethod } from './testLibrary'

/**
 * The Rust core's library storage (`store.library.*`), as the tests call it: the clock is passed
 * in, as the app's frontends pass it.
 */
const library = {
  library: (now = Date.now()) => nativeCallSync<LibrarySnapshot>('store.library.library', now),
  importLibrary: (documents: LegacyDocuments) =>
    nativeCallSync<LibrarySnapshot>('store.library.importLibrary', documents),
  studyCommand: (command: StudyCommand, now = Date.now()) =>
    nativeCallSync<StudyCommandResult>('store.library.studyCommand', command, now),
  saveArchivedGame: (game: ArchivedGame): void => {
    nativeCallSync('store.library.saveArchivedGame', game)
  },
  removeArchivedGame: (id: string): void => {
    nativeCallSync('store.library.removeArchivedGame', id)
  },
  addMistakes: (key: string, color?: string, now = Date.now()) =>
    nativeCallSync<{ added: number; items: MistakeExercise[] }>(
      'store.library.addMistakes',
      key,
      color ?? null,
      now,
    ),
  answerMistake: (id: string, solved: boolean, now = Date.now()) =>
    nativeCallSync<MistakeExercise[]>('store.library.answerMistake', id, solved, now),
  saveSession: (kind: SessionKind, session: unknown): void => {
    nativeCallSync('store.library.saveSession', kind, session)
  },
  joinedTournaments: (now = Date.now()) =>
    nativeCallSync<JoinedTournament[]>('store.library.joinedTournaments', now),
  recordRepertoireMiss: (key: string, fen: string) =>
    nativeCallSync<RepertoireMisses>('store.library.recordRepertoireMiss', key, fen),
  clearRepertoireMisses: (key: string) =>
    nativeCallSync<RepertoireMisses>('store.library.clearRepertoireMisses', key),
}

let directory: string | undefined
let database: TestDatabase | undefined
let cleared = -1

/** Renderer tests talk to the real core library, on a temporary database cleared for each test. */
function ready(): void {
  if (!directory) {
    directory = mkdtempSync(join(tmpdir(), 'kchess-library-'))
    database = useTestDatabase({ dataDir: directory })
  }
  if (cleared !== testGeneration()) {
    database!.exec('DELETE FROM documents; DELETE FROM archived_games;')
    cleared = testGeneration()
  }
}

afterAll(closeLibrary)

async function closeLibrary(): Promise<void> {
  if (!directory) return
  await closeNativeCore()
  rmSync(directory, { recursive: true, force: true })
  directory = undefined
}

/** The core's library methods, validated as the core service validates them. */
export function libraryApi(): Pick<CoreApi, LibraryMethod> {
  const run = <T>(work: () => T): Promise<T> => {
    try {
      ready()
      return Promise.resolve(work())
    } catch (cause) {
      return Promise.reject(cause instanceof Error ? cause : new Error(String(cause)))
    }
  }
  return {
    library: () => run(() => library.library()),
    importLibrary: (documents) =>
      run(() => library.importLibrary(assertLegacyDocuments(documents))),
    studyCommand: (command) =>
      run(() => library.studyCommand(assertStudyCommand(assertStudyCommandShape(command)))),
    saveArchivedGame: (game) =>
      run(() => library.saveArchivedGame(assertArchivedGame(assertArchivedGameShape(game)))),
    removeArchivedGame: (id) => run(() => library.removeArchivedGame(id)),
    addMistakes: (key, color) => run(() => library.addMistakes(key, color)),
    answerMistake: (id, solved) => run(() => library.answerMistake(id, solved)),
    saveSession: (kind, session) =>
      run(() => {
        const checked = assertSessionKind(kind)
        library.saveSession(checked, assertSession(checked, session))
      }),
    joinedTournaments: () => run(() => library.joinedTournaments()),
    recordRepertoireMiss: (key, fen) => run(() => library.recordRepertoireMiss(key, fen)),
    clearRepertoireMisses: (key) => run(() => library.clearRepertoireMisses(key)),
  }
}

/** What the core holds now, as a later app start would load it. */
export const storedLibrary = () => {
  ready()
  return library.library()
}

/**
 * A document saved by an earlier release, as the app finds it at its next start: imported by the
 * core and loaded into the snapshot stores start from.
 */
export function seedSaved(key: keyof LegacyDocuments, text: string): void {
  ready()
  database!.prepare('DELETE FROM documents WHERE key = ?').run('library:imported')
  setInitialLibrary(library.importLibrary({ [key]: text }))
}

/** Quit and start the app again: pending saves finish, and new stores load what the core kept. */
export async function restart(): Promise<void> {
  const pinia = getActivePinia()
  if (pinia) disposePinia(pinia)
  await flushPromises()
  setInitialLibrary(storedLibrary())
  setActivePinia(createPinia())
}
