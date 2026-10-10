import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it, vi } from 'vitest'
import { useTestPlatform } from '../../../tests/fixtures/corePlatform'
import { DEFAULT_SETTINGS } from '../../src/contracts/defaultSettings'
import type { ReviewStatus, ReviewUpdate, StoredReview } from '../../src/contracts/types'

// The bundled Stockfish is found relative to the app; the database goes to a scratch folder.
const userData = mkdtempSync(join(tmpdir(), 'kchess-review-'))
useTestPlatform({ dataDir: userData })
const { requestReview, setupReviews, stopReviews, FULL_DEPTH } =
  await import('../../src/services/review')
const { closeNativeCore } = await import('../../src/services/nativeCore')
const { analyseReview } = await import('../../src/domain/review')

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
/** A short game with a clear blunder: 3. Qh5 Nf6?? 4. Qxf7#, so Black's third move loses. */
const SCHOLARS = ['e2e4', 'e7e5', 'f1c4', 'b8c6', 'd1h5', 'g8f6', 'h5f7']

const updates: ReviewUpdate[] = []
const statuses: ReviewStatus[] = []
setupReviews({
  settings: async () => ({ ...DEFAULT_SETTINGS, reviewAuto: 'off' }),
  accounts: async () => [],
  onBattery: () => false,
  busy: () => undefined,
  fetchLichess: async () => [],
  update: (update) => void updates.push(update),
  status: (status) => void statuses.push(status),
})

afterAll(async () => {
  stopReviews()
  await closeNativeCore()
  rmSync(userData, { recursive: true, force: true })
})

function until(test: (review: StoredReview) => boolean, ms: number): Promise<StoredReview> {
  return vi.waitFor(
    () => {
      const found = updates.map((u) => u.review).find(test)
      if (!found) throw new Error('not yet')
      return found
    },
    { timeout: ms, interval: 50 },
  )
}

describe('game review (bundled Stockfish)', () => {
  it('shows a quick pass first, then a deep one that finds the blunder', async () => {
    expect(requestReview({ fen: START, moves: SCHOLARS })).toBeNull()
    const quick = await until((r) => r.evals.filter(Boolean).length === SCHOLARS.length + 1, 20_000)
    expect(quick.complete).toBe(false)
    const started = Date.now()
    const full = await until((r) => r.complete, 60_000)
    console.info(`deep pass: ${SCHOLARS.length + 1} positions in ${Date.now() - started} ms`)
    expect(full.evals.slice(0, -1).every((e) => (e?.depth ?? 0) >= FULL_DEPTH - 2)).toBe(true)
    // Checkmate needs no engine.
    expect(full.evals.at(-1)).toEqual({ mate: 0 })
    const analysis = analyseReview(full)
    expect(analysis.moves[5]?.judgment).toBe('blunder')
    expect(analysis.black.blunder).toBe(1)
    // The engine's choice instead of Nf6 defends f7.
    expect(full.evals[5]?.best).toMatch(/^(d8e7|d8f6|g7g6)$/)
    expect(statuses.at(-1)?.current).toBeUndefined()
  }, 90_000)

  it('answers from the store once a game is reviewed', () => {
    const stored = requestReview({ fen: START, moves: SCHOLARS })
    expect(stored?.complete).toBe(true)
  })
})
