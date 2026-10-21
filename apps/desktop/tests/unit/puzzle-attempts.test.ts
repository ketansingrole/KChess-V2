import { describe, expect, it } from 'vitest'
import type { Puzzle, PuzzleActivityEntry, PuzzleAttempt } from '@kchess/contracts/types'
import {
  HISTORY_MODES,
  moveAccuracy,
  playedText,
  puzzleHistoryRows,
  rowMatches,
} from '../../app/utils/puzzleAttempts'

const puzzle = (id: string): Puzzle => ({
  id,
  fen: '8/8/8/8/8/8/8/8 w - - 0 1',
  solution: ['e2e4'],
  rating: 1500,
  themes: [],
})
const local = (id: number, puzzleId: string, at: number): PuzzleAttempt => ({
  id,
  at,
  day: '2026-10-19',
  puzzle: puzzle(puzzleId),
  mode: 'practice',
  retry: false,
  outcome: 'partial',
  hinted: false,
  found: 2,
  wrong: 1,
  total: 5,
})
const lichess = (puzzleId: string, date: number, win: boolean): PuzzleActivityEntry => ({
  date,
  win,
  puzzle: puzzle(puzzleId),
})

describe('puzzle history rows', () => {
  it('merges local and Lichess attempts newest first, each attempt once', () => {
    const rows = puzzleHistoryRows(
      [local(2, 'aaa', 10 * 60_000), local(1, 'bbb', 0)],
      [
        // The same practice attempt, as Lichess recorded it a moment later.
        lichess('aaa', 10 * 60_000 + 5_000, false),
        lichess('ccc', 30 * 60_000, true),
        // The same puzzle played a day earlier is a different attempt.
        lichess('bbb', -86_400_000, true),
      ],
    )
    expect(rows.map((row) => [row.source, row.puzzle.id])).toEqual([
      ['lichess', 'ccc'],
      ['local', 'aaa'],
      ['local', 'bbb'],
      ['lichess', 'bbb'],
    ])
    expect(rows[1]).toMatchObject({ moves: '2/5 moves · 1 wrong', badge: { label: 'Partial' } })
    expect(rows[0]!.moves).toBeUndefined()
    expect(rows[0]!.badge.label).toBe('Solved')
  })

  it('keeps a local attempt’s tried moves for studying it on the board', () => {
    const tried = { ...local(4, 'eee', 5), wrongMoves: [{ index: 0, uci: 'd2d4' }] }
    const [row] = puzzleHistoryRows([tried], [lichess('fff', 0, false)])
    expect(row!.wrongMoves).toEqual([{ index: 0, uci: 'd2d4' }])
  })

  it('reports move accuracy only once a move was tried', () => {
    expect(moveAccuracy(0, 0)).toBeUndefined()
    expect(moveAccuracy(8, 10)).toBe(80)
  })
})

// History rows all looked alike: Practice, Downloaded, Rated, retries and Lichess-only attempts.
describe('puzzle history modes', () => {
  it('names how each attempt was played and filters retries with their mode', () => {
    const retry = { ...local(3, 'ddd', 2), retry: true, mode: 'rated' as const }
    const rows = puzzleHistoryRows([retry, local(1, 'bbb', 1)], [lichess('ccc', 0, true)])
    expect(rows.map((row) => row.mode)).toEqual(['retry', 'practice', 'lichess'])
    expect(rows.filter((row) => rowMatches(row, 'rated')).map((row) => row.key)).toEqual([
      'local-3',
    ])
    expect(rows.filter((row) => rowMatches(row, 'all'))).toHaveLength(3)
    expect(HISTORY_MODES.lichess.tip).toContain('only the result')
  })

  it('shows the time for today and yesterday, the date before', () => {
    const now = new Date(2026, 9, 19, 15, 0)
    expect(playedText(new Date(2026, 9, 19, 9, 5).getTime(), now)).not.toMatch(/Oct|Yesterday/)
    expect(playedText(new Date(2026, 9, 18, 22, 0).getTime(), now)).toMatch(/^Yesterday /)
    expect(playedText(new Date(2026, 9, 14, 12, 0).getTime(), now)).toMatch(/Oct/)
    expect(playedText(new Date(2025, 9, 14, 12, 0).getTime(), now)).toMatch(/2025/)
  })
})
