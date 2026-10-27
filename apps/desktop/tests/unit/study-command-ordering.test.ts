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

it('does not replay an obsolete failed save over newer study content at quit', async () => {
  const { api } = desktop()
  const studies = useStudyStore()
  const id = await studies.save('Study', '1. e4 *')
  const backend = api.studyCommand
  api.studyCommand = vi.fn(backend).mockRejectedValueOnce(new Error('Disk unavailable'))
  await expect(studies.save('Study', '1. d4 *', id)).rejects.toThrow('Disk unavailable')
  await studies.save('Study', '1. c4 *', id)
  await studies.flush(true)
  expect((await api.library()).studies[0]!.pgn).toContain('c4')
})

it('does not replay failed edits to a study or chapter removed since', async () => {
  const { api } = desktop()
  const studies = useStudyStore()
  const id = await studies.saveChapters('Study', [
    { name: 'One', pgn: '1. e4 *' },
    { name: 'Two', pgn: '1. d4 *' },
  ])
  const two = studies.items[0]!.chapters[1]!.id
  const other = await studies.save('Other', '1. Nf3 *')
  const backend = api.studyCommand
  const studyCommand = vi.fn(backend)
  studyCommand.mockRejectedValueOnce(new Error('Disk unavailable'))
  studyCommand.mockRejectedValueOnce(new Error('Disk unavailable'))
  api.studyCommand = studyCommand
  await expect(studies.saveChapter(id, two, '1. d4 d5 *')).rejects.toThrow()
  await expect(studies.saveChapter(other, '', '1. Nf3 d5 *')).rejects.toThrow()
  await studies.removeChapter(id, two)
  await studies.remove(other)
  const calls = studyCommand.mock.calls.length
  await studies.flush(true)
  expect(studyCommand).toHaveBeenCalledTimes(calls)
  const library = await api.library()
  expect(library.studies.map((study) => study.name)).toEqual(['Study'])
  expect(library.studies[0]!.chapters).toHaveLength(1)
})

it('does not replay a failed save that was still pending when newer content was queued', async () => {
  const { api } = desktop()
  const studies = useStudyStore()
  const id = await studies.save('Study', '1. e4 *')
  const backend = api.studyCommand
  const older = deferred<StudyCommandResult>()
  api.studyCommand = vi.fn(backend).mockImplementationOnce(() => older.promise)
  const olderSave = studies.save('Study', '1. d4 *', id)
  const olderFailed = expect(olderSave).rejects.toThrow('Disk unavailable')
  await flushPromises()
  const newerSave = studies.save('Study', '1. c4 *', id)
  older.reject(new Error('Disk unavailable'))
  await olderFailed
  await newerSave
  expect((await api.library()).studies[0]!.pgn).toContain('c4')
  await studies.flush(true)
  expect((await api.library()).studies[0]!.pgn).toContain('c4')
})

it('does not replay an overlapping failed chapter autosave over the newer one at quit', async () => {
  const { api } = desktop()
  const studies = useStudyStore()
  const id = await studies.save('Study', '1. e4 *')
  const chapter = studies.items[0]!.chapters[0]!.id
  const backend = api.studyCommand
  const older = deferred<StudyCommandResult>()
  api.studyCommand = vi.fn(backend).mockImplementationOnce(() => older.promise)
  const olderSave = studies.saveChapter(id, chapter, '1. e4 e5 { Older notes } *')
  const olderFailed = expect(olderSave).rejects.toThrow('Disk unavailable')
  await flushPromises()
  const newerSave = studies.saveChapter(id, chapter, '1. e4 e5 { Latest notes } *')
  older.reject(new Error('Disk unavailable'))
  await olderFailed
  await newerSave
  await studies.flush(true)
  expect((await api.library()).studies[0]!.chapters[0]!.pgn).toContain('Latest notes')
})

it('recovers from consecutive failed saves without replaying either at quit', async () => {
  const { api } = desktop()
  const studies = useStudyStore()
  const id = await studies.save('Study', '1. e4 *')
  const backend = api.studyCommand
  const studyCommand = vi.fn(backend)
  studyCommand.mockRejectedValueOnce(new Error('Disk unavailable'))
  studyCommand.mockRejectedValueOnce(new Error('Disk unavailable'))
  api.studyCommand = studyCommand
  const first = expect(studies.save('Study', '1. d4 *', id)).rejects.toThrow('Disk unavailable')
  const second = expect(studies.save('Study', '1. Nf3 *', id)).rejects.toThrow('Disk unavailable')
  const recovered = studies.save('Study', '1. c4 *', id)
  await first
  await second
  await recovered
  const calls = studyCommand.mock.calls.length
  await studies.flush(true)
  expect(studyCommand).toHaveBeenCalledTimes(calls)
  expect((await api.library()).studies[0]!.pgn).toContain('c4')
  expect(studies.error).toBe('')
})

it('keeps another window’s study rename when this window autosaves the chapter', async () => {
  // The content save sent this window's older name with the moves, and the core stored both.
  const { api } = desktop()
  const store = useStudyStore()
  const id = await store.save('Original', '1. e4 *')
  const chapter = store.items[0]!.chapters[0]!.id
  store.track(id, chapter)
  await api.studyCommand({ op: 'rename', id, name: 'Remote rename' })
  await store.saveChapter(id, chapter, '1. e4 e5 *')
  const [saved] = (await api.library()).studies
  expect(saved!.name).toBe('Remote rename')
  expect(saved!.chapters[0]!.pgn).toBe('1. e4 e5 *')
})

it('does not recreate a study another window removed when this window autosaves it', async () => {
  // The save recreated the study with a new chapter id, so every later save was refused.
  const { api } = desktop()
  const store = useStudyStore()
  const id = await store.save('Original', '1. e4 *')
  const chapter = store.items[0]!.chapters[0]!.id
  store.track(id, chapter)
  await api.studyCommand({ op: 'remove', id })
  await expect(store.saveChapter(id, chapter, '1. e4 e5 *')).rejects.toThrow(
    'This study was changed in another KChess window.',
  )
  expect((await api.library()).studies).toEqual([])
  // Refused like any other window's change: the board keeps a copy, and quitting is not blocked.
  await store.flush(true)
})

it('does not send a creation again at quit when only its reply was lost', async () => {
  // The core stored the duplicate, then the connection closed before its reply: the quit flush
  // duplicated the study again.
  const { api } = desktop()
  const store = useStudyStore()
  const id = await store.save('Original', '1. e4 *')
  const backend = api.studyCommand
  let lose = true
  api.studyCommand = async (...args) => {
    const result = await backend(...args)
    if (lose) {
      lose = false
      throw new Error('The KChess core stopped.')
    }
    return result
  }
  await expect(store.duplicate(id)).rejects.toThrow('core stopped')
  expect((await api.library()).studies).toHaveLength(2)
  await store.flush(true)
  expect((await api.library()).studies).toHaveLength(2)
})
