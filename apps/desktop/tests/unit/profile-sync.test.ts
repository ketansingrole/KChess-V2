import { afterEach, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { desktop, deferred } from './fixtures'
import { useGameArchiveStore } from '../../app/stores/gameArchive'
import { setLibraryRefresher } from '../../app/utils/library'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import type { ArchivedGame, LibrarySnapshot } from '@kchess/rules/library'
import type { ProfileChange } from '@kchess/contracts/types'
import profileSyncPlugin from '../../app/plugins/profileSync.client'

// The plugin module calls `defineNuxtPlugin` as it loads; hoisted above the imports.
vi.hoisted(() => {
  ;(globalThis as Record<string, unknown>).defineNuxtPlugin = (setup: unknown) => setup
})

const game = (id: string): ArchivedGame => ({
  id,
  source: 'board',
  setup: STANDARD_SETUP,
  moves: ['e2e4'],
  result: '*',
  white: 'White',
  black: 'Black',
  reason: 'In progress',
  timeControl: '',
  finished: false,
  startedAt: 1,
  updatedAt: 2,
})

/** Another window's change, as the core reports it (`plugins/profileSync.client.ts`). */
function profileSync(api: ReturnType<typeof desktop>['api']) {
  let changed: (change: ProfileChange) => void = () => {}
  api.onProfileChanged = (callback) => {
    changed = callback
    return () => {}
  }
  ;(profileSyncPlugin as unknown as () => void)()
  return (areas: ProfileChange['areas']) => changed({ areas, origin: 99 })
}

afterEach(() => setLibraryRefresher(undefined))

it('shows the newest library read when replies arrive out of order', async () => {
  const { api } = desktop()
  const replies = [deferred<LibrarySnapshot>(), deferred<LibrarySnapshot>()]
  const real = api.library
  let reads = 0
  api.library = () => replies[reads++]!.promise
  const changed = profileSync(api)
  changed(['playedGames'])
  changed(['playedGames'])
  const library = await real()
  replies[1]!.resolve({ ...library, games: [game('newer')] })
  await flushPromises()
  replies[0]!.resolve({ ...library, games: [game('older')] })
  await flushPromises()
  expect(useGameArchiveStore().games.map((value) => value.id)).toEqual(['newer'])
})

it('shows another window’s game once this window’s own save settles', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  const archive = useGameArchiveStore()
  const write = api.saveArchivedGame
  const gate = deferred<undefined>()
  api.saveArchivedGame = async (value) => {
    await gate.promise
    await write(value)
  }
  const changed = profileSync(api)
  archive.save(game('local'))
  const flushing = archive.flush()
  await flushPromises()
  // Another window saves a game while this one's is being written.
  await write(game('remote'))
  changed(['playedGames'])
  await flushPromises()
  gate.resolve(undefined)
  await flushing
  await flushPromises()
  expect(archive.games.map((value) => value.id).sort()).toEqual(['local', 'remote'])
})
