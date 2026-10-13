import { expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import { flushPromises } from '@vue/test-utils'
import { useStudyStore } from '../../app/stores/studies'
import { useAnalysisStore } from '../../app/stores/analysis'
import { desktop, deferred } from './fixtures'
import { useLocalGameStore } from '../../app/stores/local'
import { useKChessStore } from '../../app/stores/kchess'
import { useGameArchiveStore } from '../../app/stores/gameArchive'
import {
  flushSavedDocuments,
  persistSession,
  registerPersistenceFlush,
} from '../../app/utils/library'
import type { Settings } from '@kchess/contracts/types'

it('flushes the latest game, archive and identity before their debounce fires', async () => {
  desktop()
  const game = useLocalGameStore()
  game.start()
  expect(game.move('e2e4')).toBe(true)
  await flushSavedDocuments()
  const library = await window.kchess.library()
  expect(library.sessions.local?.moves).toEqual(['e2e4'])
  expect(library.games).toHaveLength(1)
  expect(library.sessions['archive:board']?.id).toBe(library.games[0]?.id)
})
it('flushes a pending settings change without waiting for its timer', async () => {
  const save = vi.fn(async (settings: Settings) => settings)
  desktop({ saveSettings: save })
  const store = useKChessStore()
  await store.init()
  store.settings!.appearance = 'dark'
  await flushSavedDocuments()
  expect(save).toHaveBeenCalledWith(expect.objectContaining({ appearance: 'dark' }))
})
it('detaches persistence callbacks when their scope is disposed', async () => {
  const saveSession = vi.fn(async () => {})
  desktop({ saveSession })
  const scope = effectScope()
  scope.run(() =>
    persistSession('analysis', () => ({
      pgn: '*',
      path: '',
      orientation: 'white',
      study: '',
      chapter: '',
    })),
  )
  scope.stop()
  const count = saveSession.mock.calls.length
  await flushSavedDocuments()
  expect(saveSession).toHaveBeenCalledTimes(count)
})
it('flushes remaining owners even if one save rejects', async () => {
  const failed = registerPersistenceFlush(async () => {
    throw new Error('Failed save')
  })
  const done = vi.fn(async () => {})
  const unregister = registerPersistenceFlush(done)
  try {
    await expect(flushSavedDocuments()).rejects.toThrow('Some changes could not be saved')
    expect(done).toHaveBeenCalledOnce()
  } finally {
    failed()
    unregister()
  }
})

it('keeps a failed session open and saves its latest contents on retry', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  const backend = api.saveSession
  api.saveSession = async () => {
    throw new Error('Disk unavailable')
  }
  const document = ref({
    pgn: '1. e4 *',
    path: '',
    orientation: 'white' as const,
    study: '',
    chapter: '',
  })
  const scope = effectScope()
  const persistence = scope.run(() => persistSession('analysis', () => document.value))!
  try {
    await expect(flushSavedDocuments()).rejects.toThrow('Some changes could not be saved')
    expect(persistence.value).toBe('Disk unavailable')
    document.value.pgn = '1. e4 e5 *'
    api.saveSession = backend
    await flushSavedDocuments()
    expect((await api.library()).sessions.analysis?.pgn).toBe('1. e4 e5 *')
    expect(persistence.value).toBe('')
  } finally {
    scope.stop()
  }
})

it('reports a failed study flush and preserves the edited tree for retry', async () => {
  vi.useFakeTimers()
  const { api } = desktop({ reviewGet: async () => null })
  const studies = useStudyStore()
  const id = await studies.save('Study', '1. e4 *')
  const analysis = useAnalysisStore()
  await analysis.openStudy(id)
  analysis.root.comments = ['Keep this edit']
  const backend = api.studyCommand
  api.studyCommand = async () => {
    throw new Error('Disk unavailable')
  }
  await expect(flushSavedDocuments()).rejects.toThrow('Some changes could not be saved')
  expect(analysis.root.comments).toEqual(['Keep this edit'])
  expect(analysis.studySaveError).toBe('Disk unavailable')
  api.studyCommand = backend
  await flushSavedDocuments()
  expect((await api.library()).studies[0]!.pgn).toContain('Keep this edit')
})

it('retains a failed archived game for retry before completing quit preparation', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  const backend = api.saveArchivedGame
  api.saveArchivedGame = async () => {
    throw new Error('Disk unavailable')
  }
  const game = useLocalGameStore()
  game.start()
  game.move('e2e4')
  await expect(flushSavedDocuments()).rejects.toThrow('Some changes could not be saved')
  api.saveArchivedGame = backend
  await flushSavedDocuments()
  expect((await api.library()).games[0]!.moves).toEqual(['e2e4'])
})

it('waits for an in-flight archive failure and retains that game for retry', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  const backend = api.saveArchivedGame
  const gate = deferred<undefined>()
  api.saveArchivedGame = () => gate.promise
  const game = useLocalGameStore()
  game.start()
  game.move('e2e4')
  await nextTick()
  const writing = useGameArchiveStore().flush()
  const quitting = expect(flushSavedDocuments()).rejects.toThrow('Some changes could not be saved')
  await flushPromises()
  gate.reject(new Error('Disk unavailable'))
  await Promise.all([writing, quitting])
  api.saveArchivedGame = backend
  await flushSavedDocuments()
  expect((await api.library()).games[0]!.moves).toEqual(['e2e4'])
})

it('settles other owners even when a flush throws synchronously', async () => {
  const failed = registerPersistenceFlush(() => {
    throw new Error('Failed save')
  })
  const done = vi.fn(async () => {})
  const unregister = registerPersistenceFlush(done)
  try {
    await expect(flushSavedDocuments()).rejects.toThrow('Some changes could not be saved')
    expect(done).toHaveBeenCalledOnce()
  } finally {
    failed()
    unregister()
  }
})

it('flushes the selected study chapter before its debounce fires', async () => {
  vi.useFakeTimers()
  desktop({ reviewGet: async () => null, onReviewUpdate: () => () => {} })
  const studies = useStudyStore()
  const id = await studies.saveChapters('Study', [
    { name: 'First', pgn: '1. e4 *' },
    { name: 'Second', pgn: '1. d4 *' },
  ])
  const analysis = useAnalysisStore()
  await analysis.openStudy(id, studies.items[0]!.chapters[1]!.id)
  analysis.root.comments = ['Latest edit']
  await nextTick()
  await flushSavedDocuments()
  const library = await window.kchess.library()
  expect(library.studies[0]!.chapters[1]!.pgn).toContain('Latest edit')
  expect(library.studies[0]!.chapters[0]!.pgn).toBe('1. e4 *')
})

it('waits for in-flight study saves even when their PGN is already marked saved', async () => {
  vi.useFakeTimers()
  const { api } = desktop({ reviewGet: async () => null, onReviewUpdate: () => () => {} })
  const studies = useStudyStore()
  const id = await studies.save('Study', '1. e4 *')
  const analysis = useAnalysisStore()
  await analysis.openStudy(id)
  const backend = api.studyCommand
  const gate = deferred<undefined>()
  api.studyCommand = async (...args) => {
    await gate.promise
    return backend(...args)
  }
  analysis.root.comments = ['Pending edit']
  await nextTick()
  const saving = analysis.flushStudy()
  await flushPromises()
  let finished = false
  const quit = flushSavedDocuments().then(() => {
    finished = true
  })
  await flushPromises()
  expect(finished).toBe(false)
  gate.resolve(undefined)
  await Promise.all([saving, quit])
  expect((await api.library()).studies[0]!.pgn).toContain('Pending edit')
})

it('drains queued study commands before completing quit preparation', async () => {
  const { api } = desktop()
  const studies = useStudyStore()
  const id = await studies.save('Study', '1. e4 *')
  const backend = api.studyCommand
  const gate = deferred<undefined>()
  api.studyCommand = async (...args) => {
    await gate.promise
    return backend(...args)
  }
  const renamed = studies.rename(id, 'Renamed')
  const chapter = studies.addChapter(id, 'New chapter')
  let finished = false
  const quit = flushSavedDocuments().then(() => {
    finished = true
  })
  await flushPromises()
  expect(finished).toBe(false)
  gate.resolve(undefined)
  await Promise.all([renamed, chapter, quit])
  const library = await api.library()
  expect(library.studies[0]!.name).toBe('Renamed')
  expect(library.studies[0]!.chapters).toHaveLength(2)
})
