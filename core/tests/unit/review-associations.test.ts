import { DatabaseSync } from 'node:sqlite'
import { afterAll, beforeEach, expect, it, vi } from 'vitest'
import { INITIAL_FEN } from 'chessops/fen'
import { migrate, MIGRATIONS } from '../../src/services/migrations'
import {
  writeReview,
  readReview,
  reviewSummaries,
  gamesToReview,
} from '../../src/services/reviewStore'
import { insights } from '../../src/services/insights'
import { reviewKey } from '../../src/domain/review'
import type { StoredReview } from '../../src/contracts/types'
import { requestReview, stopReviews } from '../../src/services/review'

const db = new DatabaseSync(':memory:')
migrate(db)
db.exec('PRAGMA foreign_keys=ON')
vi.mock('../../src/services/db', () => ({ getDb: () => db }))
vi.mock('../../src/services/engine', () => ({
  engineIdentity: vi.fn(),
  engineStatus: vi.fn(),
  spawnEngine: vi.fn(),
}))
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
beforeEach(() => db.exec('DELETE FROM game_reviews; DELETE FROM reviews; DELETE FROM games'))
afterAll(() => {
  stopReviews()
  db.close()
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
it('links a requesting game before returning a cached Lichess review', () => {
  writeReview({ ...review, source: 'lichess', gameId: 'Game0001' })
  expect(requestReview({ fen: INITIAL_FEN, moves, gameId: 'Game0002' })?.gameId).toBe('Game0002')
  expect(Object.keys(reviewSummaries(['Game0001', 'Game0002']))).toHaveLength(2)
})
it('retains both associations when identical searches are queued before their first output', () => {
  requestReview({ fen: INITIAL_FEN, moves, gameId: 'Game0001' })
  requestReview({ fen: INITIAL_FEN, moves, gameId: 'Game0002' })
  writeReview({ ...review, gameId: 'Game0002' })
  expect(Object.keys(reviewSummaries(['Game0001', 'Game0002']))).toHaveLength(2)
})
it('backfills associations from the preceding schema without discarding evaluations', () => {
  const legacy = new DatabaseSync(':memory:')
  const backfill = MIGRATIONS.findIndex((sql) => sql.includes('CREATE TABLE game_reviews'))
  for (const sql of MIGRATIONS.slice(0, backfill)) legacy.exec(sql)
  legacy.exec(`PRAGMA user_version=${backfill}`)
  legacy
    .prepare('INSERT INTO reviews VALUES (?,?,?,?,?,?,?,?)')
    .run(review.key, 'Game0001', 'local', 1, 18, JSON.stringify(review), '{}', 1)
  migrate(legacy)
  expect(legacy.prepare('SELECT * FROM game_reviews').all()).toEqual([
    { gameId: 'Game0001', reviewKey: review.key },
  ])
  expect(legacy.prepare('SELECT data FROM reviews').get()?.data).toBe(JSON.stringify(review))
  migrate(legacy)
  legacy.close()
})
