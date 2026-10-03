import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import { DatabaseSync } from 'node:sqlite'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { migrate, MIGRATIONS } from '../../src/main/migrations'
import type { LichessGame } from '../../src/shared/types'
import { assertGamePageQuery } from '../../src/shared/validate'
import { ratingHistoryFromGames } from '../../app/utils/ratings'
import {
  clearAccountData,
  gameLibraryOverview,
  gamePage,
  gamePgn,
  gameRatingHistory,
  loadData,
  MAX_GAMES,
  pendingGameIds,
  removeAccount,
  RESULT_SQL,
  saveGames,
  saveGamesPage,
} from '../../src/main/store'
import { syncGames } from '../../src/main/lichess'
import { safeStorage } from 'electron'

const state = vi.hoisted(() => ({ db: null as DatabaseSync | null, fetch: vi.fn() }))
vi.mock('electron', () => ({ shell: {}, app: {}, safeStorage: {} }))
vi.mock('../../src/main/db', () => ({ getDb: () => state.db! }))
vi.mock('../../src/main/usage', () => ({
  withUsage: (_account: string, _kind: string, fn: () => unknown) => fn(),
  attributeTo: () => {},
}))
vi.mock('../../src/main/requestPolicy', () => ({
  lichessFetch: (request: Request) => state.fetch(request),
}))

function game(id: string, overrides: Partial<LichessGame> = {}): LichessGame {
  return {
    id,
    account: 'Alice',
    createdAt: Date.UTC(2026, 9, 1),
    lastMoveAt: Date.UTC(2026, 9, 1),
    rated: true,
    speed: 'blitz',
    perf: 'blitz',
    status: 'mate',
    color: 'white',
    winner: 'white',
    opponent: 'Bob',
    playerRating: 1500,
    ratingDiff: 8,
    moves: 'e4 e5',
    pgn: '1. e4 e5 *',
    ...overrides,
  }
}
function raw(status: string) {
  return {
    id: 'Pending1',
    createdAt: Date.UTC(2026, 9, 3, 11, 40),
    lastMoveAt: Date.now(),
    rated: true,
    speed: 'blitz',
    perf: 'blitz',
    status,
    variant: 'standard',
    winner: status === 'mate' ? 'white' : undefined,
    players: {
      white: {
        user: { name: 'Alice' },
        rating: 1500,
        ratingDiff: status === 'mate' ? 8 : undefined,
      },
      black: { user: { name: 'Bob' }, rating: 1490 },
    },
    moves: status === 'mate' ? 'e4 e5 Nf3' : 'e4',
  }
}
function response(rows: unknown[]) {
  return new Response(rows.map((row) => JSON.stringify(row)).join('\n') + '\n', {
    headers: { 'Content-Type': 'application/x-ndjson' },
  })
}
beforeEach(async () => {
  state.db = new DatabaseSync(':memory:')
  state.db.exec('PRAGMA foreign_keys = ON')
  migrate(state.db)
  state.db.exec("INSERT INTO accounts (username, connected) VALUES ('Alice', 1), ('Bob', 0)")
  await saveGames('Alice', [], 1)
  state.fetch.mockReset()
})
afterEach(() => state.db?.close())

describe('SQLite game library', () => {
  it('transfers bounded pages, applies combined filters and reads PGN on demand', async () => {
    const games = Array.from({ length: 240 }, (_, i) =>
      game(`G${i.toString().padStart(7, '0')}`, {
        rated: i % 2 === 0,
        winner: i % 3 === 0 ? 'black' : 'white',
        createdAt: i,
      }),
    )
    await saveGames('Alice', games)
    const data = await loadData()
    expect(data.gameCount).toBe(240)
    expect(data).not.toHaveProperty('games')
    const query = { account: 'alice', result: 'loss' as const, rated: true, offset: 0, limit: 10 }
    const first = await gamePage(query)
    const next = await gamePage({ ...query, offset: 10 })
    expect(first.total).toBe(40)
    expect(first.games).toHaveLength(10)
    expect(new Set([...first.games, ...next.games].map((g) => g.id)).size).toBe(20)
    expect(first.games.every((g) => g.rated && g.winner !== g.color && g.pgn === undefined)).toBe(
      true,
    )
    expect(await gamePgn('ALICE', first.games[0]!.id)).toBe('1. e4 e5 *')
    expect((await gamePage({ offset: 0, limit: 100 })).games).toHaveLength(100)
    for (const invalid of [
      { limit: 101, offset: 0 },
      { limit: 10, offset: -1 },
      { limit: 10, offset: 0, result: 'other' },
      { limit: 10, offset: 0, account: "Alice' OR 1=1" },
    ])
      expect(() => assertGamePageQuery(invalid)).toThrow()
  })
  it('uses stable tie ordering and keeps aggregates and daily rating fallback equivalent', async () => {
    const games = [
      game('Abcd0001'),
      game('Abcd0002', { winner: 'black', ratingDiff: -8, createdAt: Date.UTC(2026, 9, 1, 23) }),
      game('Abcd0003', { winner: undefined, rated: false }),
      game('Abcd0004', {
        account: 'Bob',
        opponent: 'Alice',
        color: 'black',
        winner: 'black',
        perf: 'rapid',
      }),
    ]
    await saveGames('Alice', games.slice(0, 3))
    await saveGames('Bob', games.slice(3))
    const overview = await gameLibraryOverview()
    expect(overview.byAccount.alice).toEqual({ total: 3, win: 1, loss: 1, draw: 1 })
    expect(overview.versus.bob).toEqual({ total: 3, win: 1, loss: 1, draw: 1 })
    expect(overview.versus.alice).toBeUndefined()
    expect(await gameRatingHistory('alice')).toEqual(ratingHistoryFromGames(games.slice(0, 3)))
    const first = await gamePage({ offset: 0, limit: 2 })
    const next = await gamePage({ offset: 2, limit: 2 })
    expect(new Set([...first.games, ...next.games].map((g) => g.account + g.id)).size).toBe(4)
    await clearAccountData('Alice')
    expect((await loadData()).gameCount).toBe(1)
    expect((await gameLibraryOverview()).byAccount.alice).toBeUndefined()
  })
  it('saves pending changes atomically, excludes live games, and cleans them up with accounts', async () => {
    await saveGamesPage('Alice', [game('Pending1', { status: 'started' })])
    expect(await pendingGameIds('alice')).toEqual(['Pending1'])
    expect((await loadData()).gameCount).toBe(0)
    // A broken row rolls back the whole page, including removal from pending.
    await expect(
      saveGamesPage('Alice', [
        game('Pending1'),
        game('Broken01', { createdAt: undefined as unknown as number }),
      ]),
    ).rejects.toThrow()
    expect(await pendingGameIds('Alice')).toEqual(['Pending1'])
    expect((await gamePage({ offset: 0, limit: 20 })).games).toEqual([])
    await clearAccountData('Alice')
    expect(await pendingGameIds('Alice')).toEqual([])
    await saveGamesPage('Alice', [game('Pending2', { status: 'created' })])
    await removeAccount('Alice')
    expect(await pendingGameIds('Alice')).toEqual([])
    await expect(saveGamesPage('Alice', [game('Pending2')])).rejects.toThrow(/removed/)
  })
  it('retains pending IDs when the database is closed and reopened', async () => {
    const directory = mkdtempSync(join(tmpdir(), 'kchess-pending-'))
    const path = join(directory, 'kchess.db')
    state.db!.close()
    state.db = new DatabaseSync(path)
    try {
      migrate(state.db)
      state.db.exec("INSERT INTO accounts (username, connected) VALUES ('Alice', 1)")
      await saveGames('Alice', [game('Pending1', { status: 'started' })], 1000)
      state.db.close()
      state.db = new DatabaseSync(path)
      expect(await pendingGameIds('Alice')).toEqual(['Pending1'])
      await saveGames('Alice', [game('Pending1')], 2000)
      expect(await pendingGameIds('Alice')).toEqual([])
      expect((await gamePage({ offset: 0, limit: 20 })).games[0]?.id).toBe('Pending1')
    } finally {
      state.db?.close()
      state.db = null
      rmSync(directory, { recursive: true, force: true })
    }
  })
  it('trims to the retention cap once per completed sync, not on every page', async () => {
    const games = Array.from({ length: MAX_GAMES + 1 }, (_, i) =>
      game(`T${i.toString().padStart(7, '0')}`, { createdAt: i + 1 }),
    )
    await saveGamesPage('Alice', games)
    expect((await loadData()).gameCount).toBe(MAX_GAMES + 1)
    await saveGames('Alice', [], 3000)
    expect((await loadData()).gameCount).toBe(MAX_GAMES)
    expect(await gamePgn('Alice', 'T0000000')).toBeNull()
  })
  it('serves result filters from the result indexes', () => {
    const plan = (sql: string, ...params: string[]): string =>
      state
        .db!.prepare(`EXPLAIN QUERY PLAN ${sql}`)
        .all(...params)
        .map((row) => String(row.detail))
        .join(' | ')
    expect(plan(`SELECT COUNT(*) FROM games WHERE (${RESULT_SQL}) = ?`, 'loss')).toContain(
      'idx_games_result_page',
    )
    expect(
      plan(
        `SELECT COUNT(*) FROM games WHERE account = ? COLLATE NOCASE AND (${RESULT_SQL}) = ?`,
        'alice',
        'loss',
      ),
    ).toContain('idx_games_account_result_page')
  })
  it('appends a one-time backfill migration without losing completed games', () => {
    const db = new DatabaseSync(':memory:')
    try {
      const backfill = MIGRATIONS.findIndex((sql) => sql.includes('CREATE TABLE pending_game_sync'))
      for (const sql of MIGRATIONS.slice(0, backfill)) db.exec(sql)
      db.exec(`PRAGMA user_version = ${backfill}`)
      db.exec("INSERT INTO accounts (username, connected, lastSyncedAt) VALUES ('Alice', 1, 1000)")
      const insert = db.prepare(
        "INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status, color, opponent, moves) VALUES ('Alice', ?, 1, 1, 1, 'blitz', 'blitz', ?, 'white', 'Bob', 'e4')",
      )
      insert.run('Pending1', 'started')
      insert.run('Finished', 'mate')
      migrate(db)
      expect(db.prepare('SELECT lastSyncedAt FROM accounts').get()?.lastSyncedAt).toBeNull()
      expect(db.prepare('SELECT id FROM pending_game_sync').get()?.id).toBe('Pending1')
      expect(db.prepare('SELECT id FROM games').get()?.id).toBe('Finished')
      db.exec('UPDATE accounts SET lastSyncedAt = 2000')
      migrate(db)
      expect(db.prepare('SELECT lastSyncedAt FROM accounts').get()?.lastSyncedAt).toBe(2000)
    } finally {
      db.close()
    }
  })
})

describe('Lichess creation-time sync', () => {
  it('recovers an older game that finishes after the cursor, retaining pending IDs through missing and failed exports', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(Date.UTC(2026, 9, 3, 12))
    // Initial/backfill sync includes ongoing games explicitly.
    state.db!.exec('UPDATE accounts SET lastSyncedAt = NULL')
    state.fetch.mockImplementation(async (request: Request) => {
      expect(new URL(request.url).searchParams.get('ongoing')).toBe('true')
      return response([raw('started')])
    })
    await syncGames('Alice')
    const cursor = (await loadData()).accounts[0]!.lastSyncedAt
    expect(await pendingGameIds('Alice')).toEqual(['Pending1'])
    expect((await loadData()).gameCount).toBe(0)
    state.fetch.mockImplementation(async (request: Request) =>
      request.method === 'POST' ? response([]) : response([]),
    )
    await syncGames('Alice')
    expect(await pendingGameIds('Alice')).toEqual(['Pending1'])
    state.fetch.mockResolvedValue(new Response('unavailable', { status: 503 }))
    await expect(syncGames('Alice')).rejects.toThrow()
    expect((await loadData()).accounts[0]!.lastSyncedAt).toBe(cursor)
    expect(await pendingGameIds('Alice')).toEqual(['Pending1'])
    vi.mocked(Date.now).mockReturnValue(Date.UTC(2026, 9, 3, 12, 10))
    state.fetch.mockImplementation(async (request: Request) => {
      if (request.method === 'POST') {
        expect(await request.text()).toBe('Pending1')
        return response([raw('mate')])
      }
      expect(Number(new URL(request.url).searchParams.get('since'))).toBe(cursor! - 60_000)
      return response([])
    })
    await syncGames('Alice')
    expect(await pendingGameIds('Alice')).toEqual([])
    const saved = (await gamePage({ offset: 0, limit: 20 })).games[0]!
    expect(saved.status).toBe('mate')
    expect(saved.moves).toBe('e4 e5 Nf3')
  })
})

describe('Lichess export authentication', () => {
  beforeEach(() => {
    Object.assign(safeStorage, {
      isEncryptionAvailable: () => true,
      getSelectedStorageBackend: () => 'keychain',
      decryptString: (buffer: Buffer) => buffer.toString(),
    })
    state.db!.exec(
      `INSERT INTO tokens (username, encrypted) VALUES ('Alice', '${Buffer.from('lip_alice').toString('base64')}')`,
    )
  })
  afterEach(() => {
    for (const key of ['isEncryptionAvailable', 'getSelectedStorageBackend', 'decryptString'])
      delete (safeStorage as unknown as Record<string, unknown>)[key]
  })

  it("sends the account's own token, and none for a friend without one", async () => {
    const seen: { url: string; auth: string | null }[] = []
    state.fetch.mockImplementation(async (request: Request) => {
      seen.push({ url: request.url, auth: request.headers.get('authorization') })
      return response([])
    })
    await syncGames()
    expect(seen.find((r) => r.url.includes('/user/Alice'))?.auth).toBe('Bearer lip_alice')
    expect(seen.find((r) => r.url.includes('/user/Bob'))?.auth).toBeNull()
  })
  it('falls back to an anonymous export when the token is rejected', async () => {
    const auths: (string | null)[] = []
    state.fetch.mockImplementation(async (request: Request) => {
      auths.push(request.headers.get('authorization'))
      return request.headers.has('authorization')
        ? new Response('{"error":"No such token"}', { status: 401 })
        : response([])
    })
    await syncGames('Alice')
    expect(auths).toEqual(['Bearer lip_alice', null])
    expect((await loadData()).accounts[0]!.lastSyncedAt).toBeGreaterThan(1)
  })
})
