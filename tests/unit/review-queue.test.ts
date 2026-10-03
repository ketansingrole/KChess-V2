import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it, vi } from 'vitest'
import { DEFAULT_SETTINGS } from '../../src/shared/defaultSettings'
import type { ReviewStatus, ReviewUpdate, Settings } from '../../src/shared/types'

const userData = mkdtempSync(join(tmpdir(), 'kchess-review-queue-'))
vi.mock('electron', () => ({
  app: { getAppPath: () => process.cwd(), getPath: () => userData },
}))
const { reviewsChanged, setupReviews, stopReviews } = await import('../../src/main/review')
const { closeDb, getDb } = await import('../../src/main/db')
const { reviewSummaries } = await import('../../src/main/reviewStore')

function addGame(id: string, moves: string, perf = 'blitz'): void {
  getDb()
    .prepare(
      `INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status, color,
        opponent, moves) VALUES ('tester', ?, ?, ?, 1, 'blitz', ?, 'mate', 'white', 'rival', ?)`,
    )
    .run(id, Date.now(), Date.now(), perf, moves)
}

const settings: Settings = { ...DEFAULT_SETTINGS, reviewAuto: 'recent' }
let busy: 'engine' | undefined
const asked: string[][] = []
const updates: ReviewUpdate[] = []
const statuses: ReviewStatus[] = []
setupReviews({
  settings: async () => settings,
  accounts: async () => ['tester'],
  onBattery: () => false,
  busy: () => busy,
  fetchLichess: async (_account, ids) => {
    asked.push(ids)
    const { markChecked } = await import('../../src/main/reviewStore')
    markChecked(ids)
    return []
  },
  update: (update) => void updates.push(update),
  status: (status) => void statuses.push(status),
})

afterAll(() => {
  stopReviews()
  closeDb()
  rmSync(userData, { recursive: true, force: true })
})

describe('automatic game review', () => {
  it('asks Lichess first, then reviews the game locally; variants are left alone', async () => {
    addGame('scholar1', 'e4 e5 Qh5 Nc6 Bc4 Nf6 Qxf7#')
    addGame('variant1', 'e4 e5', 'chess960')
    reviewsChanged()
    await vi.waitFor(
      () => {
        if (!reviewSummaries().scholar1?.complete) throw new Error('not yet')
      },
      { timeout: 60_000, interval: 100 },
    )
    expect(asked).toEqual([['scholar1']])
    expect(reviewSummaries().scholar1?.black.blunder).toBe(1)
    expect(reviewSummaries().variant1).toBeUndefined()
    expect(updates.at(-1)?.review).toMatchObject({ gameId: 'scholar1', complete: true })
  }, 90_000)

  it('waits while the engine is needed elsewhere, and is off when switched off', async () => {
    busy = 'engine'
    addGame('waiting1', 'd4 d5 c4')
    reviewsChanged()
    await vi.waitFor(() => expect(statuses.at(-1)?.paused).toBe('engine'))
    expect(reviewSummaries().waiting1).toBeUndefined()
    busy = undefined
    settings.reviewAuto = 'off'
    reviewsChanged()
    await vi.waitFor(() => expect(statuses.at(-1)?.paused).toBe('off'))
    expect(reviewSummaries().waiting1).toBeUndefined()
  })
})
