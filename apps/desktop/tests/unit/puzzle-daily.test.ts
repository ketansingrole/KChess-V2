import { afterEach, expect, it, vi } from 'vitest'
import { usePuzzleStore } from '../../app/stores/puzzles'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop, puzzle } from './fixtures'

afterEach(() => vi.useRealTimers())

it('loads a new daily puzzle once the day has changed', async () => {
  // The first daily puzzle of the session stayed after midnight.
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(new Date('2026-10-20T23:50:00Z'))
  const puzzleDaily = vi
    .fn()
    .mockResolvedValueOnce({ ...puzzle, id: 'today01' })
    .mockResolvedValueOnce({ ...puzzle, id: 'tomorrow' })
  desktop({ puzzleDaily })
  await useKChessStore().init()
  const puzzles = usePuzzleStore()
  await puzzles.loadDaily()
  await puzzles.loadDaily()
  expect(puzzleDaily).toHaveBeenCalledTimes(1)
  vi.setSystemTime(new Date('2026-10-21T00:05:00Z'))
  await puzzles.loadDaily()
  expect(puzzleDaily).toHaveBeenCalledTimes(2)
  expect(puzzles.daily?.id).toBe('tomorrow')
})
