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

it('keeps a deleted game hidden when a library read from before the deletion arrives', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  await api.saveArchivedGame(game('deleted'))
  const archive = useGameArchiveStore()
  archive.replaceFromLibrary(await api.library())
  const stale = await api.library()
  const remove = api.removeArchivedGame
  const gate = deferred<undefined>()
  api.removeArchivedGame = async (id) => {
    await gate.promise
    await remove(id)
  }
  let refreshes = 0
  setLibraryRefresher(() => {
    refreshes++
    void api.library().then((snapshot) => archive.replaceFromLibrary(snapshot))
  })
  const removing = archive.remove('deleted')
  await flushPromises()
  archive.replaceFromLibrary(stale)
  expect(archive.games.map((value) => value.id)).toEqual([])
  gate.resolve(undefined)
  await removing
  await flushPromises()
  expect(refreshes).toBe(1)
  expect(archive.games.map((value) => value.id)).toEqual([])
})

it('keeps a deleted game hidden when a library read from before the deletion arrives after it', async () => {
  const { api } = desktop()
  await api.saveArchivedGame(game('deleted'))
  const archive = useGameArchiveStore()
  archive.replaceFromLibrary(await api.library())
  const stale = await api.library()
  const real = api.library
  const gate = deferred<LibrarySnapshot>()
  const library = vi
    .spyOn(api, 'library')
    .mockImplementationOnce(() => gate.promise)
    .mockImplementation(() => real())
  const changed = profileSync(api)
  changed(['playedGames'])
  await archive.remove('deleted')
  expect(archive.games).toEqual([])
  gate.resolve(stale)
  await flushPromises()
  expect(library).toHaveBeenCalledTimes(2)
  expect(archive.games).toEqual([])
})

it('reads the profile again after reconnecting to the core', async () => {
  const { api } = desktop()
  let reconnected = () => {}
  api.onCoreReconnected = (callback) => {
    reconnected = callback
    return () => {}
  }
  profileSync(api)
  const library = vi.spyOn(api, 'library')
  const data = vi.spyOn(api, 'loadData')
  // Another window saved a game while this one was disconnected; nothing reported it.
  await api.saveArchivedGame(game('missed'))
  reconnected()
  await flushPromises()
  expect(library).toHaveBeenCalledOnce()
  expect(data).toHaveBeenCalledOnce()
  expect(useGameArchiveStore().games.map((value) => value.id)).toEqual(['missed'])
})
