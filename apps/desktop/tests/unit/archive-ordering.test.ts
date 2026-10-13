import { expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { desktop, deferred } from './fixtures'
import { useGameArchiveStore } from '../../app/stores/gameArchive'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import type { ArchivedGame } from '@kchess/rules/library'

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

it('skips a deleted batch entry while the preceding save waits', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  const archive = useGameArchiveStore()
  const write = api.saveArchivedGame
  const gate = deferred<undefined>()
  api.saveArchivedGame = async (value) => {
    if (value.id === 'first') await gate.promise
    await write(value)
  }
  archive.save(game('first'))
  archive.save(game('second'))
  const flushing = archive.flush()
  await flushPromises()
  const removing = archive.remove('second')
  gate.resolve(undefined)
  await Promise.all([flushing, removing])
  expect((await api.library()).games.map((value) => value.id)).toEqual(['first'])
  expect(archive.games.map((value) => value.id)).toEqual(['first'])
})

it('deletes after an in-flight save and preserves a newer save after deletion', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  const archive = useGameArchiveStore()
  const write = api.saveArchivedGame
  const gate = deferred<undefined>()
  const sent = deferred<undefined>()
  api.saveArchivedGame = async (value) => {
    sent.resolve(undefined)
    await gate.promise
    await write(value)
  }
  archive.save(game('same'))
  const flushing = archive.flush()
  await sent.promise
  const removing = archive.remove('same')
  gate.resolve(undefined)
  await Promise.all([flushing, removing])
  expect((await api.library()).games).toEqual([])
  archive.save({ ...game('same'), moves: ['e2e4', 'e7e5'] })
  await archive.flush()
  expect((await api.library()).games[0]?.moves).toEqual(['e2e4', 'e7e5'])
})

it('restores a saved game when deletion fails', async () => {
  vi.useFakeTimers()
  desktop()
  const archive = useGameArchiveStore()
  archive.save(game('kept'))
  await archive.flush()
  window.kchess.removeArchivedGame = async () => {
    throw new Error('Disk unavailable')
  }
  await archive.remove('kept')
  expect(archive.games.map((value) => value.id)).toEqual(['kept'])
  expect(archive.error).toBe('Disk unavailable')
})

it('drains deletion before saving a newer version of the same game', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  const archive = useGameArchiveStore()
  archive.save(game('same'))
  await archive.flush()
  const remove = api.removeArchivedGame
  const gate = deferred<undefined>()
  const entered = deferred<undefined>()
  api.removeArchivedGame = async (id) => {
    entered.resolve(undefined)
    await gate.promise
    await remove(id)
  }
  const removing = archive.remove('same')
  await entered.promise
  archive.save({ ...game('same'), moves: ['e2e4', 'e7e5'] })
  let flushed = false
  const flushing = archive.flush().then(() => {
    flushed = true
  })
  await flushPromises()
  expect(flushed).toBe(false)
  gate.resolve(undefined)
  await Promise.all([removing, flushing])
  expect((await api.library()).games[0]?.moves).toEqual(['e2e4', 'e7e5'])
  expect(archive.games[0]?.moves).toEqual(['e2e4', 'e7e5'])
})
