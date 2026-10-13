import { expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { useStudyStore } from '../../app/stores/studies'
import type { StudyCommandResult } from '@kchess/rules/library'
import { deferred, desktop } from './fixtures'
import { loadLibrary, setInitialLibrary } from '../../app/utils/library'

it('retains a successful study mutation when the next queued mutation fails, then recovers', async () => {
  const { api } = desktop()
  const backend = api.studyCommand
  const pending = deferred<StudyCommandResult>()
  const studyCommand = vi.fn(backend)
  studyCommand.mockImplementationOnce(() => pending.promise)
  studyCommand.mockRejectedValueOnce(new Error('Disk unavailable'))
  window.kchess.studyCommand = studyCommand
  const store = useStudyStore()
  const saved = store.save('Study', '1. e4 *')
  const renamed = store.rename('unknown', 'Renamed')
  const failed = expect(renamed).rejects.toThrow('Disk unavailable')
  await flushPromises()
  expect(studyCommand).toHaveBeenCalledOnce()
  pending.resolve(await backend({ op: 'save', name: 'Study', pgn: '1. e4 *' }))
  const id = await saved
  await failed
  expect(store.items.map((study) => study.name)).toEqual(['Study'])
  expect(store.error).toBe('Disk unavailable')
  await store.rename(id, 'Recovered')
  expect(store.items[0]!.name).toBe('Recovered')
  expect(store.error).toBe('')
})

it('surfaces a damaged study library at startup without claiming it is empty', () => {
  desktop()
  setInitialLibrary({
    studyError: 'The saved study library is damaged. Its data has been preserved.',
  })
  const store = useStudyStore()
  expect(store.error).toContain('preserved')
})

it('retains an earlier renderer recovery copy when the core reports damaged studies', async () => {
  const { api } = desktop()
  const snapshot = await api.library()
  const damaged = { ...snapshot, imported: false, studyError: 'Stored studies are damaged.' }
  api.library = async () => damaged
  api.importLibrary = async () => ({ ...damaged, imported: true })
  const recovery = '{"version":1,"items":[]}'
  localStorage.setItem('kchess:studies:v1', recovery)
  await loadLibrary()
  expect(localStorage.getItem('kchess:studies:v1')).toBe(recovery)
})
