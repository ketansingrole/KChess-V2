import { expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { flushPromises } from '@vue/test-utils'
import { desktop, deferred } from './fixtures'
import { useStudyStore } from '../../app/stores/studies'
import { useAnalysisStore } from '../../app/stores/analysis'

/** The study bar's delete: remember the board's link, then delete the open study. */
async function deleteOpenStudy() {
  vi.useFakeTimers()
  const { api } = desktop({ reviewGet: async () => null, onReviewUpdate: () => () => {} })
  const studies = useStudyStore()
  const analysis = useAnalysisStore()
  const id = await studies.save('Study', '1. e4 { Original notes } *')
  await analysis.openStudy(id)
  const relink = analysis.captureStudyLink()
  const removed = (await studies.remove(id))!
  await nextTick()
  expect(analysis.studyId).toBe('')
  return { api, studies, analysis, id, relink, removed }
}

const savedPgn = async (api: ReturnType<typeof desktop>['api']) =>
  (await api.library()).studies[0]!.chapters[0]!.pgn

it('restores a deleted study without linking the different game loaded since', async () => {
  const { api, studies, analysis, relink, removed } = await deleteOpenStudy()
  analysis.loadPgn('1. d4 { Different document } *')
  await studies.restore(removed)
  relink()
  await vi.advanceTimersByTimeAsync(300)
  await analysis.flushStudy()
  expect(analysis.studyId).toBe('')
  const pgn = await savedPgn(api)
  expect(pgn).toContain('e4')
  expect(pgn).toContain('Original notes')
  expect(pgn).not.toContain('Different document')
})

it('does not link a game loaded while the restore is pending', async () => {
  const { api, studies, analysis, relink, removed } = await deleteOpenStudy()
  const backend = api.studyCommand
  const gate = deferred<undefined>()
  api.studyCommand = vi.fn(backend).mockImplementationOnce(async (...args) => {
    await gate.promise
    return backend(...args)
  })
  const restoring = studies.restore(removed).then(relink)
  await flushPromises()
  analysis.loadPgn('1. d4 { Different document } *')
  gate.resolve(undefined)
  await restoring
  await vi.advanceTimersByTimeAsync(300)
  await analysis.flushStudy()
  expect(analysis.studyId).toBe('')
  expect(await savedPgn(api)).toContain('Original notes')
  expect(await savedPgn(api)).not.toContain('Different document')
})

it('relinks the board that still holds the restored study and keeps saving its edits', async () => {
  const { api, studies, analysis, id, relink, removed } = await deleteOpenStudy()
  await studies.restore(removed)
  relink()
  expect(analysis.studyId).toBe(id)
  analysis.root.comments = ['After undo']
  await vi.advanceTimersByTimeAsync(300)
  await analysis.flushStudy()
  const pgn = await savedPgn(api)
  expect(pgn).toContain('Original notes')
  expect(pgn).toContain('After undo')
})
