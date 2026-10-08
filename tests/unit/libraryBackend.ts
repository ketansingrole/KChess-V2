import { afterAll } from 'vitest'
import { createPinia, disposePinia, getActivePinia, setActivePinia } from 'pinia'
import { flushPromises } from '@vue/test-utils'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import * as library from '../../src/core/library'
import { closeDb, getDb } from '../../src/core/db'
import { setPlatform } from '../../src/core/platform'
import {
  assertArchivedGame,
  assertLegacyDocuments,
  assertSession,
  assertSessionKind,
  assertStudyCommand,
  type LegacyDocuments,
} from '../../src/shared/library'
import type { CoreApi } from '../../src/shared/types'
import { setInitialLibrary } from '../../app/utils/library'
import { testPlatform } from './corePlatform'
import { testGeneration, type LibraryMethod } from './testLibrary'

let directory: string | undefined
let cleared = -1

/** Renderer tests talk to the real core library, on a temporary database cleared for each test. */
function ready(): void {
  if (!directory) {
    directory = mkdtempSync(join(tmpdir(), 'kchess-library-'))
    setPlatform(testPlatform({ dataDir: directory }))
  }
  if (cleared !== testGeneration()) {
    getDb().exec('DELETE FROM documents; DELETE FROM archived_games;')
    cleared = testGeneration()
  }
}

afterAll(closeLibrary)

function closeLibrary(): void {
  if (!directory) return
  closeDb()
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
    studyCommand: (command) => run(() => library.studyCommand(assertStudyCommand(command))),
    saveArchivedGame: (game) => run(() => library.saveArchivedGame(assertArchivedGame(game))),
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
  getDb().prepare('DELETE FROM documents WHERE key = ?').run('library:imported')
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
