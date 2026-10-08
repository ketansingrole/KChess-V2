import type { CoreApi } from '../../src/shared/types'
import { setInitialLibrary } from '../../app/utils/library'

/**
 * Library state shared by every test. It imports no core module: setup files run before a
 * suite registers its mocks, and suites that mock core modules (db, store, usage, …) must load
 * them afterwards. `libraryBackend` (via `desktop()`) provides the real library.
 */
const METHODS = [
  'library',
  'importLibrary',
  'studyCommand',
  'saveArchivedGame',
  'removeArchivedGame',
  'addMistakes',
  'answerMistake',
  'saveSession',
  'joinedTournaments',
  'recordRepertoireMiss',
  'clearRepertoireMisses',
] as const
export type LibraryMethod = (typeof METHODS)[number]

let generation = 0
/** Bumped for every test: the backend clears its database on the first call in each. */
export const testGeneration = (): number => generation

/** A fresh, empty library for the next test; stores start from `snapshot`. */
export function resetLibrary(snapshot?: Parameters<typeof setInitialLibrary>[0]): void {
  generation++
  setInitialLibrary(snapshot)
}

/**
 * The default for every test: unfinished sessions and played games are accepted and not kept;
 * anything that needs the library itself fails until the test asks for one with `desktop()`.
 */
export function detachedLibraryApi(): Pick<CoreApi, LibraryMethod> {
  const missing = async () => {
    throw new Error('This test has no library: call desktop() first.')
  }
  return {
    ...(Object.fromEntries(METHODS.map((method) => [method, missing])) as unknown as Pick<
      CoreApi,
      LibraryMethod
    >),
    saveSession: async () => {},
    saveArchivedGame: async () => {},
    removeArchivedGame: async () => {},
  }
}
