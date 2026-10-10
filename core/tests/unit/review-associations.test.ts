import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeEach, expect, it } from 'vitest'
import { INITIAL_FEN } from '../../src/domain/position'
import { closeNativeCore, nativeCall, nativeCallSync } from '../../src/services/nativeCore'
import { useTestDatabase, type TestDatabase } from '../../../tests/fixtures/nativeStore'
import { reviewKey } from '../../src/domain/review'
import type { StoredReview } from '../../src/contracts/types'

// The review associations are the store's: these tests write and read them through the core's
// storage methods, on one temporary profile with no engine (they never start a search).
const writeReview = (review: StoredReview) =>
  nativeCallSync<{ review: StoredReview; summary: { complete: boolean } }>(
    'store.reviewStore.writeReview',
    review,
  )
const readReview = (key: string) =>
  nativeCallSync<StoredReview | null>('store.reviewStore.readReview', key)
const reviewSummaries = (ids: string[]) =>
  nativeCallSync<Record<string, { complete: boolean }>>('store.reviewStore.reviewSummaries', ids)
const gamesToReview = (accounts: string[]) =>
  nativeCallSync<unknown[]>('store.reviewStore.gamesToReview', accounts, 0, 300)
const insights = (query: { account: string }) =>
  nativeCallSync<{ accuracy?: { games: number } }>('store.insights.insights', query)
const requestReview = (request: { fen: string; moves: string[]; gameId: string }) =>
  nativeCall<StoredReview | null>('reviews.request', request)

const dataDir = mkdtempSync(join(tmpdir(), 'kchess-associations-'))
let db: TestDatabase
const moves = ['e2e4', 'e7e5']
const review: StoredReview = {
  key: reviewKey(INITIAL_FEN, moves),
  fen: INITIAL_FEN,
  moves,
  source: 'local',
  complete: true,
  depth: 18,
  updatedAt: 1,
  evals: [{ cp: 0 }, { cp: 0 }, { cp: 0 }],
}
beforeEach(async () => {
  await closeNativeCore()
  // No engine: these tests only read and write associations, never a search.
  db = useTestDatabase({ dataDir, bundledEnginePath: join(dataDir, 'no-engine.js') })
  db.exec('DELETE FROM game_reviews; DELETE FROM reviews; DELETE FROM games')
})
afterAll(async () => {
  await nativeCall('reviews.stop')
  await closeNativeCore()
  rmSync(dataDir, { recursive: true, force: true })
})
function game(id: string, account = 'Alice') {
  db.prepare(
    "INSERT INTO games (account,id,createdAt,lastMoveAt,rated,speed,perf,status,color,opponent,moves) VALUES (?, ?, 1,1,0,'blitz','blitz','resign','white','Bob','e4 e5')",
  ).run(account, id)
}
it('retains all associations through updates and excludes each account copy from the queue', () => {
  game('Game0001')
  game('Game0002')
  game('Game0001', 'Bob')
  writeReview({ ...review, gameId: 'Game0001' })
  writeReview({ ...review, gameId: 'Game0002' })
  writeReview({ ...review, gameId: 'Game0002', depth: 20 })
  expect(Object.keys(reviewSummaries(['Game0001', 'Game0002']))).toEqual(['Game0001', 'Game0002'])
  expect(gamesToReview(['Alice', 'Bob'])).toEqual([])
  expect(insights({ account: 'Alice' }).accuracy?.games).toBe(2)
})
it('links a second game even when a stronger review prevents replacing its evaluations', () => {
  writeReview({ ...review, source: 'lichess', gameId: 'Game0001' })
  const result = writeReview({ ...review, complete: false, gameId: 'Game0002' })
  expect(result.review).toMatchObject({ source: 'lichess', complete: true, gameId: 'Game0002' })
  expect(readReview(review.key)?.source).toBe('lichess')
  expect(reviewSummaries(['Game0001', 'Game0002']).Game0002?.complete).toBe(true)
})
it('links a requesting game before returning a cached Lichess review', async () => {
  writeReview({ ...review, source: 'lichess', gameId: 'Game0001' })
  expect((await requestReview({ fen: INITIAL_FEN, moves, gameId: 'Game0002' }))?.gameId).toBe(
    'Game0002',
  )
  expect(Object.keys(reviewSummaries(['Game0001', 'Game0002']))).toHaveLength(2)
})
it('retains both associations when identical searches are queued before their first output', async () => {
  await requestReview({ fen: INITIAL_FEN, moves, gameId: 'Game0001' })
  await requestReview({ fen: INITIAL_FEN, moves, gameId: 'Game0002' })
  writeReview({ ...review, gameId: 'Game0002' })
  expect(Object.keys(reviewSummaries(['Game0001', 'Game0002']))).toHaveLength(2)
})
