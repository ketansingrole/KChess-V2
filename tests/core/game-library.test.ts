import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { closeNativeCore } from '@kchess/native/nativeCore'
import { useTestDatabase, type TestDatabase } from '../fixtures/nativeStore'
import type { LichessGame } from '@kchess/contracts/types'
import { assertGamePageQuery } from '@kchess/rules/validate'
import { ratingHistoryFromGames } from '@kchess/rules/ratings'
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
  saveGames,
  saveGamesPage,
  saveLogin,
  writeApiCache,
  hasAccount,
} from '../fixtures/nativeGames'
import { fakeSecrets } from '../fixtures/corePlatform'

const state = vi.hoisted(() => ({ db: null as TestDatabase | null, fetch: vi.fn() }))
/** The result of a game, as the store's list filters compute it. */
const RESULT_SQL =
  "CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END"
vi.mock('../../src/services/usage', () => ({
  withUsage: (_account: string, _kind: string, fn: () => unknown) => fn(),
  attributeTo: () => {},
}))
vi.mock('../../src/services/requestPolicy', () => ({
  lichessFetch: (request: Request) => state.fetch(request),
}))
const secrets = fakeSecrets()

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
beforeEach(async () => {
  state.db = useTestDatabase({ secrets })
  state.db.exec("INSERT INTO accounts (username, connected) VALUES ('Alice', 1), ('Bob', 0)")
  await saveGames('Alice', [], 1)
  state.fetch.mockReset()
})
afterEach(async () => {
  state.db = null
  await closeNativeCore()
})

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
    try {
      await closeNativeCore()
      state.db = useTestDatabase({ dataDir: directory, secrets })
      state.db.exec("INSERT INTO accounts (username, connected) VALUES ('Alice', 1)")
      await saveGames('Alice', [game('Pending1', { status: 'started' })], 1000)
      await closeNativeCore()
      state.db = useTestDatabase({ dataDir: directory, secrets })
      expect(await pendingGameIds('Alice')).toEqual(['Pending1'])
      await saveGames('Alice', [game('Pending1')], 2000)
      expect(await pendingGameIds('Alice')).toEqual([])
      expect((await gamePage({ offset: 0, limit: 20 })).games[0]?.id).toBe('Pending1')
    } finally {
      await closeNativeCore()
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
  })
})
