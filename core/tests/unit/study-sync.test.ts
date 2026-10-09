import { beforeEach, expect, it, vi } from 'vitest'
import { syncLichessStudy } from '../../src/services/studies'
import { assertStudySyncRequest } from '../../src/domain/validate'
const mocks = vi.hoisted(() => ({ GET: vi.fn(), POST: vi.fn(), pgn: '' }))
vi.mock('../../src/services/lichess', () => ({
  client: mocks,
  asAccount: (_account: string, task: (token: string) => unknown) => task('token'),
  authorize: (token: string) => ({ Authorization: `Bearer ${token}` }),
  unwrap: async (result: unknown) => result,
  urlencoded: (body: Record<string, string>) => new URLSearchParams(body).toString(),
}))
const baseline =
  '[Site "https://lichess.org/study/Study001/Chapter1"]\n[Event "Example"]\n[Result "*"]\n\n1. e4 {Keep} e5 (1... c5 $1) *'
const request = {
  account: 'Alice',
  studyId: 'Study001',
  baseline,
  pgn: baseline.replace('{Keep}', '{Edited notes}'),
}
beforeEach(() => {
  mocks.pgn = baseline
  mocks.GET.mockReset().mockImplementation(async () => mocks.pgn)
  mocks.POST.mockReset().mockResolvedValue({ ok: true })
})
it('checks the cloud baseline before updating the same chapter, preserving PGN annotations', async () => {
  await syncLichessStudy(request)
  expect(mocks.POST.mock.calls.map(([path]) => path)).toEqual([
    '/api/study/{studyId}/{chapterId}/moves',
    '/api/study/{studyId}/{chapterId}/tags',
  ])
  const moves = mocks.POST.mock.calls[0]![1]
  expect(moves.params.path).toEqual({ studyId: 'Study001', chapterId: 'Chapter1' })
  expect(moves.body.pgn).toContain('Edited notes')
  expect(moves.body.pgn).toContain('c5 $1')
})
it('blocks upload when the cloud changed without any mutations', async () => {
  mocks.pgn = baseline.replace('Keep', 'Cloud edit')
  await expect(syncLichessStudy(request)).rejects.toThrow('cloud study changed')
  expect(mocks.POST).not.toHaveBeenCalled()
})
it('rejects illegal moves, structure changes, and wrong chapter identities before writing', async () => {
  await expect(
    syncLichessStudy({ ...request, pgn: request.pgn.replace('e4', 'e5') }),
  ).rejects.toThrow('illegal move')
  await expect(
    syncLichessStudy({ ...request, baseline: baseline + '\n\n[Event "Extra"]\n\n1. d4 *' }),
  ).rejects.toThrow('structure changed')
  const wrong = baseline.replace('/Study001/', '/Other001/')
  mocks.pgn = wrong
  await expect(syncLichessStudy({ ...request, baseline: wrong })).rejects.toThrow(
    'chapter identity',
  )
  expect(mocks.POST).not.toHaveBeenCalled()
})
it('does not retry an ambiguous mutation', async () => {
  mocks.POST.mockRejectedValueOnce(new Error('Connection lost'))
  await expect(syncLichessStudy(request)).rejects.toThrow('Connection lost')
  expect(mocks.POST).toHaveBeenCalledTimes(1)
})
it('validates the IPC request and bounds uploaded documents', () => {
  expect(assertStudySyncRequest(request)).toEqual(request)
  expect(() => assertStudySyncRequest({ ...request, studyId: 'bad' })).toThrow()
  expect(() => assertStudySyncRequest({ ...request, pgn: 'x'.repeat(500001) })).toThrow()
})

it('appends new chapters to the linked cloud study without duplicating the existing chapters', async () => {
  await syncLichessStudy({
    ...request,
    pgn: baseline + '\n\n[ChapterName "Second chapter"]\n\n1. d4 *',
  })
  expect(mocks.POST).toHaveBeenCalledTimes(1)
  expect(mocks.POST.mock.calls[0]![0]).toBe('/api/study/{studyId}/import-pgn')
  expect(mocks.POST.mock.calls[0]![1].body.name).toBe('Second chapter')
})
