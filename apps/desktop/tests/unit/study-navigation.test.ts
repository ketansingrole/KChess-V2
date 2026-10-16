import { expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { flushPromises } from '@vue/test-utils'
import { desktop, deferred } from './fixtures'
import { useStudyStore } from '../../app/stores/studies'
import { useAnalysisStore } from '../../app/stores/analysis'

it('preserves annotations across a rapid chapter round trip while saving', async () => {
  vi.useFakeTimers()
  const { api } = desktop({ reviewGet: async () => null, onReviewUpdate: () => () => {} })
  const studies = useStudyStore()
  const id = await studies.saveChapters('Study', [
    { name: 'One', pgn: '1. e4 *' },
    { name: 'Two', pgn: '1. d4 *' },
  ])
  const [one, two] = studies.items[0]!.chapters
  const analysis = useAnalysisStore()
  await analysis.openStudy(id, one!.id)
  analysis.root.comments = ['Keep my edits']
  await nextTick()
  const backend = api.studyCommand
  const gate = deferred<undefined>()
  api.studyCommand = vi.fn(async (...args: Parameters<typeof backend>) => {
    await gate.promise
    return backend(...args)
  })
  const toSecond = analysis.openStudy(id, two!.id)
  await flushPromises()
  const toFirst = analysis.openStudy(id, one!.id)
  expect(analysis.root.comments).toEqual(['Keep my edits'])
  gate.resolve(undefined)
  expect(await toSecond).toBe(false)
  expect(await toFirst).toBe(true)
  expect(analysis.pgn()).toContain('Keep my edits')
  analysis.root.comments.push('Later edit')
  await nextTick()
  await analysis.flushStudy()
  const pgn = (await api.library()).studies[0]!.chapters[0]!.pgn
  expect(pgn).toContain('Keep my edits')
  expect(pgn).toContain('Later edit')
})

it('rejects a failed flush, preserves the board on a blocked switch, and recovers on retry', async () => {
  vi.useFakeTimers()
  const { api } = desktop({ reviewGet: async () => null, onReviewUpdate: () => () => {} })
  const studies = useStudyStore()
  const id = await studies.saveChapters('Study', [
    { name: 'One', pgn: '1. e4 *' },
    { name: 'Two', pgn: '1. d4 *' },
  ])
  const [one, two] = studies.items[0]!.chapters
  const analysis = useAnalysisStore()
  await analysis.openStudy(id, one!.id)
  analysis.root.comments = ['Keep my edits']
  await nextTick()
  const backend = api.studyCommand
  api.studyCommand = vi.fn(async (command, incremental) => {
    if (command.op === 'save') throw new Error('Disk unavailable')
    return backend(command, incremental)
  })
  await expect(analysis.flushStudy()).rejects.toThrow('Disk unavailable')
  expect(await analysis.openStudy(id, two!.id)).toBe(false)
  expect(analysis.studyChapterId).toBe(one!.id)
  expect(analysis.root.comments).toEqual(['Keep my edits'])
  expect(analysis.studySaveError).toBe('Disk unavailable')
  api.studyCommand = backend
  expect(await analysis.openStudy(id, two!.id)).toBe(true)
  expect(studies.items[0]!.chapters[0]!.pgn).toContain('Keep my edits')
})

it('leaves a board loaded while "Save as study" was pending unlinked from the new study', async () => {
  const { api } = desktop({ reviewGet: async () => null, onReviewUpdate: () => () => {} })
  const analysis = useAnalysisStore()
  const backend = api.studyCommand
  const gate = deferred<undefined>()
  api.studyCommand = vi.fn(async (...args: Parameters<typeof backend>) => {
    await gate.promise
    return backend(...args)
  })
  analysis.loadPgn('1. e4 *')
  const saving = analysis.saveAsStudy('Original')
  await flushPromises()
  analysis.loadPgn('1. d4 *')
  gate.resolve(undefined)
  const id = await saving
  await analysis.flushStudy()
  expect(analysis.studyId).toBe('')
  const original = (await api.library()).studies.find((s) => s.id === id)!
  expect(original.name).toBe('Original')
  expect(original.pgn).toContain('e4')
  expect(original.pgn).not.toContain('d4')
})

it('keeps a study rename when an autosave is queued behind it', async () => {
  const { api } = desktop({ reviewGet: async () => null, onReviewUpdate: () => () => {} })
  const analysis = useAnalysisStore(),
    studies = useStudyStore()
  analysis.loadPgn('1. e4 *')
  const id = await analysis.saveAsStudy('Old name')
  const backend = api.studyCommand
  const gate = deferred<undefined>()
  api.studyCommand = vi.fn(async (...args: Parameters<typeof backend>) => {
    if (args[0].op === 'rename') await gate.promise
    return backend(...args)
  })
  const rename = studies.rename(id, 'New name')
  await flushPromises()
  analysis.root.comments = ['New notes']
  await nextTick()
  const save = analysis.flushStudy()
  gate.resolve(undefined)
  await rename
  await save
  const [study] = (await api.library()).studies
  expect(study!.name).toBe('New name')
  expect(study!.pgn).toContain('New notes')
})
