import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import { DatabaseSync } from 'node:sqlite'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { migrate, MIGRATIONS } from '../../src/core/migrations'
import type { LichessGame } from '../../src/shared/types'
import { assertGamePageQuery } from '../../src/shared/validate'
import { ratingHistoryFromGames } from '../../src/shared/ratings'
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
  logoutAccounts,
  RESULT_SQL,
  saveGames,
  saveGamesPage,
  saveLogin,
  writeApiCache,
} from '../../src/core/store'
import {
  cancelAccountSyncs,
  fetchLichessReviews,
  followedUsers,
  invalidateLogin,
  primeProfiles,
  syncGames,
} from '../../src/core/lichess'
import { hasAccount, reviewSummaries } from '../../src/core/reviewStore'
import { fakeSecrets, useTestPlatform } from './corePlatform'

const state = vi.hoisted(() => ({ db: null as DatabaseSync | null, fetch: vi.fn() }))
vi.mock('../../src/core/db', () => ({ getDb: () => state.db! }))
vi.mock('../../src/core/usage', () => ({
  withUsage: (_account: string, _kind: string, fn: () => unknown) => fn(),
  attributeTo: () => {},
}))
vi.mock('../../src/core/requestPolicy', () => ({
  lichessFetch: (request: Request) => state.fetch(request),
}))
const secrets = fakeSecrets()
useTestPlatform({ secrets })

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
/** A finished game as Lichess exports it with its own computer analysis. */
function analysed() {
  const game = raw('mate')
  const analysis = { inaccuracy: 0, mistake: 0, blunder: 0, acpl: 12, accuracy: 91 }
  return {
    ...game,
    players: {
      white: { ...game.players.white, analysis },
      black: { ...game.players.black, analysis: { ...analysis, accuracy: 84 } },
    },
    analysis: [{ eval: 30 }, { eval: 25 }, { eval: 35 }],
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
  it('logs out every connected account and erases their data while preserving public following', async () => {
    const db = state.db!
    db.exec(
      "INSERT INTO accounts (username, connected) VALUES ('Carol', 1); INSERT INTO tokens VALUES ('Alice', 'secret'), ('Carol', 'secret'); INSERT INTO api_cache VALUES ('alice:profile', '{}', 0), ('carol:rating', '[]', 0), ('bob:profile', '{}', 0); INSERT INTO usage VALUES ('alice', 'games', 2, 200, 0), ('bob', 'games', 3, 300, 0)",
    )
    await saveGames('Alice', [game('Private1')])
    await saveGames('Carol', [game('Private2', { account: 'Carol' })])
    await saveGames('Bob', [game('Public01', { account: 'Bob' })])
    await saveGamesPage('Alice', [game('Pending1', { status: 'started' })])
    db.exec(
      "INSERT INTO lichess_review_checks VALUES ('Private1', 0), ('Public01', 0); INSERT INTO reviews VALUES ('private', 'Private1', 'local', 0, 0, '{}', '{}', 0), ('shared', 'Private2', 'local', 0, 0, '{}', '{}', 0); INSERT INTO game_reviews VALUES ('Private1', 'private'), ('Private2', 'shared'), ('Public01', 'shared')",
    )
    const result = await logoutAccounts()
    expect(result.accounts).toMatchObject([{ username: 'Bob', connected: false }])
    expect(result.gameCount).toBe(1)
    expect(db.prepare('SELECT key FROM reviews').all()).toEqual([{ key: 'shared' }])
    expect(db.prepare('SELECT id FROM lichess_review_checks').all()).toEqual([{ id: 'Public01' }])
    expect(db.prepare('SELECT * FROM tokens').all()).toEqual([])
    expect(db.prepare('SELECT * FROM pending_game_sync').all()).toEqual([])
    expect(db.prepare('SELECT key FROM api_cache').all()).toEqual([{ key: 'bob:profile' }])
    expect(db.prepare('SELECT account FROM usage').all()).toEqual([{ account: 'bob' }])
    expect((await gamePage({ offset: 0, limit: 20 })).games[0]?.id).toBe('Public01')
    expect((await logoutAccounts()).accounts).toMatchObject([{ username: 'Bob', connected: false }])
  })
  it('logs out one account without deleting another connected account or a followed player', async () => {
    const db = state.db!
    db.exec(
      "INSERT INTO accounts VALUES ('Carol', 1, NULL); INSERT INTO tokens VALUES ('Alice', 'secret-a'), ('Carol', 'secret-c'); INSERT INTO api_cache VALUES ('alice:profile', '{}', 0), ('carol:profile', '{}', 0)",
    )
    await saveGames('Alice', [game('Private1')])
    await saveGames('Carol', [game('Private2', { account: 'Carol' })])
    await saveGames('Bob', [game('Public01', { account: 'Bob' })])
    const result = await logoutAccounts('aLiCe')
    expect(result.accounts.map((a) => a.username).sort()).toEqual(['Bob', 'Carol'])
    expect(result.accounts.find((a) => a.username === 'Carol')?.connected).toBe(true)
    expect(result.gameCount).toBe(2)
    expect(db.prepare('SELECT username FROM tokens').all()).toEqual([{ username: 'Carol' }])
    expect(db.prepare('SELECT key FROM api_cache').all()).toEqual([{ key: 'carol:profile' }])
    await logoutAccounts('Bob')
    expect((await loadData()).accounts).toHaveLength(2)
  })
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
    const fullPage = await gamePage({ offset: 0, limit: 100 })
    expect(fullPage.games).toHaveLength(100)
    expect(fullPage.games.every((entry) => entry.pgn === undefined)).toBe(true)
    // The actual API must retain the representative 64 KiB transfer budget.
    expect(Buffer.byteLength(JSON.stringify(fullPage))).toBeLessThanOrEqual(64 * 1024)
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

describe('Lichess analysis saved during sync', () => {
  const checked = () =>
    state
      .db!.prepare('SELECT id FROM lichess_review_checks')
      .all()
      .map((row) => row.id)

  it('saves the analysis with its page, so the list can show it straight away', async () => {
    expect((await gamePage({ offset: 0, limit: 20 })).total).toBe(0)
    state.fetch.mockImplementation(async () => response([analysed()]))
    await syncGames('Alice')
    // The cached count was dropped by the write rather than going stale.
    expect((await gamePage({ offset: 0, limit: 20 })).total).toBe(1)
    const summary = reviewSummaries(['Pending1', 'Missing1'])
    expect(Object.keys(summary)).toEqual(['Pending1'])
    expect(summary.Pending1).toMatchObject({ source: 'lichess', complete: true })
    expect(summary.Pending1!.white.accuracy).toBe(91)
    expect(checked()).toEqual(['Pending1'])
  })
  it('keeps neither the analysis nor the check when its page cannot be saved', async () => {
    state.fetch.mockImplementation(async () => {
      state.db!.exec("DELETE FROM accounts WHERE username = 'Alice'")
      return response([analysed()])
    })
    await expect(syncGames('Alice')).rejects.toThrow('removed during sync')
    expect(reviewSummaries(['Pending1'])).toEqual({})
    expect(checked()).toEqual([])
  })
})

describe('Sync cancelled by logout, removal or clearing', () => {
  const checked = () =>
    state
      .db!.prepare('SELECT id FROM lichess_review_checks')
      .all()
      .map((row) => row.id)

  it('stops the sync without saving the page it was downloading', async () => {
    state.fetch.mockImplementation(async () => {
      invalidateLogin(['Alice'])
      return response([analysed()])
    })
    await expect(syncGames('Alice')).rejects.toThrow('Sync was cancelled')
    expect((await gamePage({ offset: 0, limit: 20 })).total).toBe(0)
    expect(checked()).toEqual([])
  })
  it('stops a sync for an account removed while its games download', async () => {
    let started!: () => void
    const downloading = new Promise<void>((resolve) => (started = resolve))
    let release!: (value: Response) => void
    state.fetch.mockImplementation(() => {
      started()
      return new Promise<Response>((resolve) => (release = resolve))
    })
    const sync = syncGames('Alice')
    await downloading
    cancelAccountSyncs(['Alice'])
    await removeAccount('Alice')
    release(response([analysed()]))
    await expect(sync).rejects.toThrow('Sync was cancelled')
    expect(state.db!.prepare('SELECT id FROM games').all()).toEqual([])
    expect(checked()).toEqual([])
  })
  it('stops a sync for an account whose data is cleared while its games download', async () => {
    let started!: () => void
    const downloading = new Promise<void>((resolve) => (started = resolve))
    let release!: (value: Response) => void
    state.fetch.mockImplementation(() => {
      started()
      return new Promise<Response>((resolve) => (release = resolve))
    })
    const sync = syncGames('Alice')
    await downloading
    cancelAccountSyncs(['Alice'])
    await clearAccountData('Alice')
    release(response([analysed()]))
    await expect(sync).rejects.toThrow('Sync was cancelled')
    expect(state.db!.prepare('SELECT id FROM games').all()).toEqual([])
    // The account stays, with its sync cursor reset so the next sync starts over.
    const alice = (await loadData()).accounts.find((a) => a.username === 'Alice')
    expect(alice).toBeDefined()
    expect(alice?.lastSyncedAt).toBeUndefined()
  })
  it('re-checks the login after the last await before writing a page', async () => {
    let current = true
    const saving = saveGamesPage('Alice', [game('Late0001')], [], () => current)
    // Logout lands while the save is waiting, after the caller's own check passed.
    current = false
    await expect(saving).rejects.toThrow('Sync was cancelled')
    await expect(saveGames('Alice', [game('Late0002')], 2, () => current)).rejects.toThrow(
      'Sync was cancelled',
    )
    expect((await gamePage({ offset: 0, limit: 20 })).total).toBe(0)
  })
})

describe('Lichess export authentication', () => {
  beforeEach(() => {
    secrets.enabled = true
    state.db!.exec(
      `INSERT INTO tokens (username, encrypted) VALUES ('Alice', '${Buffer.from('lip_alice').toString('base64')}')`,
    )
  })
  afterEach(() => {
    secrets.enabled = false
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

describe('No account data comes back after logout, removal or clearing', () => {
  const rows = (sql: string) => state.db!.prepare(sql).all()
  /** Alice and Bob both played Shared01; Private1 is Alice's alone. */
  async function seed(): Promise<void> {
    await saveGames('Alice', [game('Private1'), game('Shared01')])
    await saveGames('Bob', [game('Shared01', { account: 'Bob' })])
    state.db!.exec(`
      INSERT INTO reviews VALUES ('rp', 'Private1', 'local', 1, 0, '{}', '{}', 0),
        ('rs', 'Shared01', 'local', 1, 0, '{}', '{}', 0);
      INSERT INTO game_reviews VALUES ('Private1', 'rp'), ('Shared01', 'rs');
      INSERT INTO lichess_review_checks VALUES ('Private1', 0), ('Shared01', 0);
      INSERT INTO usage VALUES ('alice', 'games', 1, 10, 0), ('bob', 'games', 1, 10, 0);
      INSERT INTO api_cache VALUES ('alice:profile', '{}', 0), ('bob:profile', '{}', 0);
      INSERT INTO position_lookups VALUES
        ('player:{"player":"ALICE","color":"white"}:fen', '{}', 0),
        ('player:{"player":"Bob","color":"white"}:fen', '{}', 0),
        ('opening:fen', '{}', 0);`)
  }
  const kept = () => ({
    reviews: rows('SELECT key FROM reviews ORDER BY key'),
    checks: rows('SELECT id FROM lichess_review_checks ORDER BY id'),
    cache: rows('SELECT key FROM api_cache ORDER BY key'),
    lookups: rows('SELECT key FROM position_lookups ORDER BY key'),
  })
  const others = {
    reviews: [{ key: 'rs' }],
    checks: [{ id: 'Shared01' }],
    cache: [{ key: 'bob:profile' }],
    lookups: [{ key: 'opening:fen' }, { key: 'player:{"player":"Bob","color":"white"}:fen' }],
  }

  it('removing an account deletes its reviews, checks, lookups and usage, sparing shared games', async () => {
    await seed()
    await removeAccount('alice')
    expect(kept()).toEqual(others)
    expect(rows('SELECT account FROM usage')).toEqual([{ account: 'bob' }])
    expect(hasAccount('Alice')).toBe(false)
  })
  it('logging out deletes the same data', async () => {
    await seed()
    await logoutAccounts('Alice')
    expect(kept()).toEqual(others)
  })
  it('clearing data deletes it too but keeps the account and its usage history', async () => {
    await seed()
    await clearAccountData('Alice')
    expect(kept()).toEqual(others)
    expect(rows("SELECT account FROM usage WHERE account = 'alice'")).toHaveLength(1)
    expect(hasAccount('Alice')).toBe(true)
  })
  it('writes no Lichess analysis downloaded across a logout', async () => {
    state.fetch.mockImplementation(async () => {
      invalidateLogin(['Alice'])
      return response([analysed()])
    })
    await expect(fetchLichessReviews('Alice', ['Pending1'])).rejects.toThrow('Sync was cancelled')
    expect(rows('SELECT key FROM reviews')).toEqual([])
    expect(rows('SELECT id FROM lichess_review_checks')).toEqual([])
  })
  it('caches no profile data once its login has ended', async () => {
    await writeApiCache('alice:profile', { username: 'Alice' }, () => false)
    expect(rows('SELECT key FROM api_cache')).toEqual([])
  })

  describe('with OS encryption', () => {
    beforeEach(() => {
      secrets.enabled = true
    })
    afterEach(() => {
      secrets.enabled = false
    })

    it('stores a login and its account together, or neither after a logout', async () => {
      await expect(saveLogin('Carol', 'lip_carol', () => false)).rejects.toThrow('cancelled')
      expect(rows("SELECT username FROM tokens WHERE username = 'Carol'")).toEqual([])
      expect(hasAccount('Carol')).toBe(false)
      const data = await saveLogin('Carol', 'lip_carol', () => true)
      expect(data.accounts.find((a) => a.username === 'Carol')?.connected).toBe(true)
      expect(rows("SELECT username FROM tokens WHERE username = 'Carol'")).toHaveLength(1)
    })
    it('forgets who an account follows when it logs out during the download', async () => {
      state.db!.exec(
        `INSERT INTO tokens (username, encrypted) VALUES ('Alice', '${Buffer.from('lip_alice').toString('base64')}')`,
      )
      state.fetch.mockImplementation(async () => {
        invalidateLogin(['Alice'])
        return response([{ id: 'carol', username: 'Carol' }])
      })
      await expect(followedUsers()).rejects.toThrow('cancelled by logout')
      await primeProfiles(['Carol'])
      expect(rows("SELECT key FROM api_cache WHERE key LIKE 'carol:%'")).toEqual([])
    })
  })
})
