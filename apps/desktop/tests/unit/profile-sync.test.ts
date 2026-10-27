import { afterEach, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { desktop, deferred } from './fixtures'
import { useGameArchiveStore } from '../../app/stores/gameArchive'
import { useStudyStore } from '../../app/stores/studies'
import { useKChessStore } from '../../app/stores/kchess'
import { useUsageStore } from '../../app/stores/usage'
import { setLibraryRefresher } from '../../app/utils/library'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import type { ArchivedGame, LibrarySnapshot } from '@kchess/rules/library'
import type { AppData, ProfileChange } from '@kchess/contracts/types'
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

it('keeps a study rename when a library read from before it arrives, and autosaves the new name', async () => {
  const { api } = desktop()
  const studies = useStudyStore()
  const id = await studies.save('Original', '1. e4 *')
  const chapter = studies.items[0]!.chapters[0]!.id
  studies.track(id, chapter)
  const stale = await api.library()
  const real = api.library
  const gate = deferred<LibrarySnapshot>()
  const library = vi
    .spyOn(api, 'library')
    .mockImplementationOnce(() => gate.promise)
    .mockImplementation(() => real())
  const changed = profileSync(api)
  changed(['studies'])
  await studies.rename(id, 'Renamed')
  gate.resolve(stale)
  await flushPromises()
  expect(library).toHaveBeenCalledTimes(2)
  expect(studies.items[0]!.name).toBe('Renamed')
  await studies.saveChapter(id, chapter, '1. e4 e5 *')
  expect((await real()).studies[0]!.name).toBe('Renamed')
})

it('keeps a removed account gone when an account read from before the removal arrives', async () => {
  const { api } = desktop()
  const old = await api.loadData()
  old.accounts[1]!.connected = false
  const removed: AppData = { ...old, accounts: old.accounts.slice(0, 1) }
  const app = useKChessStore()
  api.loadData = async () => old
  await app.reloadData()
  vi.spyOn(useUsageStore(), 'refresh').mockResolvedValue(null)
  const gate = deferred<AppData>()
  const load = vi
    .fn<() => Promise<AppData>>()
    .mockImplementationOnce(() => gate.promise)
    .mockImplementation(async () => removed)
  api.loadData = load
  api.removeAccount = async () => removed
  const reading = app.reloadData()
  await app.removeAccount('Bob')
  gate.resolve(old)
  await reading
  expect(load).toHaveBeenCalledTimes(2)
  expect(app.data!.accounts.map((account) => account.username)).toEqual(['Alice'])
})

it('does not read the library when another window saves only a session', async () => {
  const { api } = desktop()
  const changed = profileSync(api)
  const library = vi.spyOn(api, 'library')
  changed(['sessions'])
  await flushPromises()
  expect(library).not.toHaveBeenCalled()
})

it('shows an earlier change’s store when a newer read for another store replaces its reply', async () => {
  const { api } = desktop()
  const replies = [deferred<LibrarySnapshot>(), deferred<LibrarySnapshot>()]
  const real = api.library
  let reads = 0
  api.library = () => replies[reads++]!.promise
  const changed = profileSync(api)
  changed(['playedGames'])
  changed(['drills'])
  const library = await real()
  replies[1]!.resolve({ ...library, games: [game('remote')] })
  await flushPromises()
  replies[0]!.resolve({ ...library, games: [] })
  await flushPromises()
  expect(useGameArchiveStore().games.map((value) => value.id)).toEqual(['remote'])
})

it('shows another window’s game from the read a stale reply starts', async () => {
  // The stale reply started a corrective read, then cleared the stores it was for: that read
  // found the other window's game but showed nothing.
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
  await api.saveArchivedGame(game('remote'))
  await archive.remove('deleted')
  gate.resolve(stale)
  await flushPromises()
  expect(library).toHaveBeenCalledTimes(2)
  expect(archive.games.map((value) => value.id)).toEqual(['remote'])
})

it('keeps a duplicated study once, and shows another window’s study it had not read', async () => {
  // The duplicate was stored, but the unread study in its reply failed it: it was queued for
  // replay, duplicated again at quit, and blocked quitting.
  const { api } = desktop()
  profileSync(api)
  const studies = useStudyStore()
  const id = await studies.save('Local', '1. e4 *')
  await api.studyCommand({ op: 'save', name: 'Remote', pgn: '1. d4 *' })
  const copy = await studies.duplicate(id)
  expect(copy).toBeTruthy()
  await studies.flush(true)
  await flushPromises()
  expect((await api.library()).studies).toHaveLength(3)
  expect(studies.items.map((study) => study.name).sort()).toEqual(
    (await api.library()).studies.map((study) => study.name).sort(),
  )
  expect(studies.error).toBe('')
})

it('keeps another window’s settings when an account change answers with them', async () => {
  // The account reply's settings became the saved base while the form kept the older value, so
  // the next unrelated save wrote the older value back.
  vi.useFakeTimers()
  const { api } = desktop()
  const old = await api.loadData()
  old.accounts[1]!.connected = false
  api.loadData = async () => old
  const app = useKChessStore()
  await app.init()
  vi.spyOn(useUsageStore(), 'refresh').mockResolvedValue(null)
  const remote = old.settings.soundVolume === 0.8 ? 0.6 : 0.8
  api.removeAccount = async () => ({
    ...old,
    settings: { ...old.settings, soundVolume: remote },
    accounts: old.accounts.slice(0, 1),
  })
  await app.removeAccount('Bob')
  expect(app.settings!.soundVolume).toBe(remote)
  app.settings!.zenMode = !app.settings!.zenMode
  const saved = vi.spyOn(api, 'saveSettings')
  await app.save()
  expect(saved).toHaveBeenCalledWith(
    expect.objectContaining({ soundVolume: remote }),
    expect.objectContaining({ soundVolume: remote }),
  )
  expect(app.data!.settings.soundVolume).toBe(remote)
  vi.useRealTimers()
})
