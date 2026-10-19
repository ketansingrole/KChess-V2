import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { testPlatform } from '../fixtures/corePlatform'
import { createKChessCore, type KChessCore } from '@kchess/native/service'
import type { PuzzleAttemptInput } from '@kchess/contracts/types'

/** The local training log: `puzzles.db`, owned by the Rust puzzle service. */
let core: KChessCore
let dataDir: string
beforeEach(() => {
  dataDir = mkdtempSync(join(tmpdir(), 'kchess-attempts-'))
  core = createKChessCore(testPlatform({ dataDir }))
})
afterEach(async () => {
  await core.close()
  rmSync(dataDir, { recursive: true, force: true })
})

const attempt = (overrides: Partial<PuzzleAttemptInput> = {}): PuzzleAttemptInput => ({
  puzzle: {
    id: 'Ab12c',
    fen: 'r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3',
    lastMove: 'b8c6',
    solution: ['f1b5', 'a7a6', 'b5c6'],
    rating: 1520,
    themes: ['opening', 'short'],
  },
  mode: 'practice',
  retry: false,
  win: true,
  revealed: false,
  hinted: false,
  found: 2,
  wrong: 0,
  total: 2,
  day: '2026-10-19',
  ...overrides,
})

describe('puzzle attempt log', () => {
  it('keeps attempts, totals and the day streak across a restart', async () => {
    await core.recordPuzzleAttempt(attempt({ day: '2026-10-17' }))
    await core.recordPuzzleAttempt(attempt({ day: '2026-10-18', hinted: true }))
    const progress = await core.recordPuzzleAttempt(
      attempt({ win: false, found: 1, wrong: 1, total: 2, mode: 'offline' }),
    )
    expect(progress.recent[0]).toMatchObject({ outcome: 'partial', found: 1, total: 2 })
    expect(progress.dayStreak).toBe(2)

    await core.close()
    core = createKChessCore(testPlatform({ dataDir }))
    const reopened = await core.puzzleProgress({ today: '2026-10-19', limit: 10 })
    expect(reopened.recent.map((entry) => entry.outcome)).toEqual(['partial', 'hinted', 'solved'])
    expect(reopened.recent[0]!.puzzle).toEqual(attempt().puzzle)
    expect(reopened.dayStreak).toBe(2)
    expect(reopened.bestDayStreak).toBe(2)
    expect(reopened.today).toMatchObject({ attempts: 1, partial: 1, movesFound: 1, movesTried: 2 })
    expect(reopened.all).toMatchObject({ attempts: 3, solved: 1, hinted: 1, movesFound: 5 })
    // A day with no solve breaks the run.
    expect((await core.puzzleProgress({ today: '2026-10-21', limit: 1 })).dayStreak).toBe(0)
  })

  it('survives deleting the downloaded puzzles', async () => {
    await core.recordPuzzleAttempt(attempt())
    await core.puzzleDbDelete()
    const progress = await core.puzzleProgress({ today: '2026-10-19', limit: 10 })
    expect(progress.all.attempts).toBe(1)
  })

  it('refuses malformed attempts and queries', async () => {
    await expect(core.recordPuzzleAttempt(attempt({ day: '2026-02-30' }))).rejects.toThrow(
      'Invalid puzzle attempt.',
    )
    await expect(core.recordPuzzleAttempt(attempt({ mode: 'storm' as never }))).rejects.toThrow(
      'Invalid puzzle attempt.',
    )
    await expect(core.puzzleProgress({ today: 'today', limit: 10 })).rejects.toThrow(
      'Invalid puzzle history query.',
    )
  })
})
