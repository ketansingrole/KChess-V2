import { copyFileSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { afterAll, describe, expect, it } from 'vitest'
import { closeDb, getDb } from '../../src/services/db'
import { MIGRATIONS, migrate } from '../../src/services/migrations'
import { nativeRules, type NativeCoreHandle } from '../../src/services/native'
import * as store from '../../src/services/store'
import { fakeSecrets, useTestPlatform } from '../../../tests/fixtures/corePlatform'

/**
 * `store.ts` (`store.games.*` in Rust, `crates/kchess-core/src/store/games.rs`) and the schema
 * (`migrations.ts`, `crates/kchess-core/src/store/migrations.rs`), checked side by side. Each
 * step runs one call on the TypeScript store (its own temporary profile) and on the Rust core
 * (another), compares the results, then compares every table, the schema and `user_version`.
 * Timestamps the two sides stamp with their own clocks are masked after a check that they are
 * current; everything else must match exactly.
 */

const root = mkdtempSync(join(tmpdir(), 'kchess-store-parity-'))
afterAll(() => rmSync(root, { recursive: true, force: true }))

const TIME_KEYS = new Set(['fetchedAt', 'dismissedAt', 'updatedAt', 'checkedAt', 'importedAt'])

function messageOf(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause)
}

/** Timestamps written by either side at the current moment read as `<now>`; any other value stays. */
function maskTimes(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(maskTimes)
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, inner]) => [
        key,
        TIME_KEYS.has(key) && typeof inner === 'number' && Math.abs(inner - Date.now()) < 60_000
          ? '<now>'
          : maskTimes(inner),
      ]),
    )
  }
  return value
}

/** The result as JSON, as a host would see it (`undefined` is `null`). */
function asJson(value: unknown): unknown {
  if (value instanceof Set) return [...value]
  if (value === undefined) return null
  return JSON.parse(JSON.stringify(value))
}

type Outcome = { ok: unknown } | { error: string }

async function tsOutcome(run: () => unknown): Promise<Outcome> {
  try {
    return { ok: asJson(await run()) }
  } catch (cause) {
    return { error: messageOf(cause) }
  }
}

function openRust(dataDir: string): NativeCoreHandle {
  const native = nativeRules()
  if (!native) throw new Error('The native rules are not built (pnpm run build:native).')
  return new native.NativeCore(
    { dataDir },
    () => {},
    () => {},
  )
}

/** One call on a freshly opened Rust core, closed again so its files can be read. */
async function rustOutcome(dataDir: string, method: string, args: unknown[]): Promise<Outcome> {
  const core = openRust(dataDir)
  try {
    return { ok: JSON.parse(core.callSync(method, JSON.stringify(args))) as unknown }
  } catch (cause) {
    return { error: messageOf(cause) }
  } finally {
    await core.close()
  }
}

interface TableDump {
  schema: unknown[]
  userVersion: unknown
  tables: Record<string, unknown[]>
}

/** Every schema object, `user_version`, and every table's rows in a total order. */
function dumpDatabase(db: DatabaseSync): TableDump {
  const schema = db
    .prepare('SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name')
    .all()
  const userVersion = db.prepare('PRAGMA user_version').get()?.user_version
  const tables: Record<string, unknown[]> = {}
  for (const entry of schema) {
    if (entry.type !== 'table' || String(entry.name).startsWith('sqlite_')) continue
    const name = String(entry.name)
    const columns = (db.prepare(`PRAGMA table_info("${name}")`).all() as { name: string }[]).map(
      (column) => `"${column.name}"`,
    )
    tables[name] = maskTimes(
      db.prepare(`SELECT * FROM "${name}" ORDER BY ${columns.join(', ')}`).all(),
    ) as unknown[]
  }
  return { schema: asJson(schema) as unknown[], userVersion, tables }
}

/** The dump of the TypeScript profile's database, through the connection the store holds. */
function dumpTs(): TableDump {
  return dumpDatabase(getDb())
}

function dumpRust(dataDir: string): TableDump {
  const db = new DatabaseSync(join(dataDir, 'kchess.db'))
  try {
    return dumpDatabase(db)
  } finally {
    db.close()
  }
}

function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const ACCOUNTS = ['Alice', 'alice', 'Bob', 'carol', 'Dave_1', 'eve-2', 'Frank%x']
const PLAYERS = ['Alice', 'ALICE', 'bob', 'Carol', 'Zed', 'Dave_1']
/** Cache keys written and read; the account prefix is what logout and removal clear. */
const API_KEYS = ['alice:profile', 'bob:ratings', 'carol:profile', 'dave_1:profile', 'other']
const STATUSES = ['mate', 'resign', 'draw', 'outoftime', 'started', 'created', 'aborted']
const SPEEDS = ['bullet', 'blitz', 'rapid', 'classical']

type Random = () => number
const pick = <T>(random: Random, items: readonly T[]): T =>
  items[Math.floor(random() * items.length)] as T
const int = (random: Random, low: number, high: number): number =>
  low + Math.floor(random() * (high - low + 1))

function randomGame(random: Random): Record<string, unknown> {
  const status = pick(random, STATUSES)
  const createdAt = int(random, 1, 40) * 86_400_000 + int(random, 0, 90_000)
  const game: Record<string, unknown> = {
    id: `G${String(int(random, 0, 40)).padStart(4, '0')}`,
    account: pick(random, ACCOUNTS),
    createdAt,
    lastMoveAt: createdAt + int(random, 0, 1000),
    rated: random() < 0.7,
    speed: pick(random, SPEEDS),
    perf: pick(random, SPEEDS),
    status,
    color: random() < 0.5 ? 'white' : 'black',
    opponent: pick(random, PLAYERS),
    moves: 'e4 e5 Nf3',
  }
  if (random() < 0.8 && status !== 'started' && status !== 'created')
    game.winner = random() < 0.5 ? 'white' : 'black'
  if (random() < 0.8) game.playerRating = int(random, 1200, 1900)
  if (random() < 0.8) game.ratingDiff = int(random, -30, 30)
  if (random() < 0.7) game.opponentRating = int(random, 1200, 1900)
  if (random() < 0.5) game.opening = pick(random, ['Italian Game', "King's Gambit"])
  if (random() < 0.5) game.pgn = '1. e4 e5 2. Nf3 *'
  // A rare broken row: a missing creation time, which the table rejects.
  if (random() < 0.04) delete game.createdAt
  return game
}

function randomSettings(random: Random): Record<string, unknown> {
  return {
    appearance: pick(random, ['system', 'light', 'dark']),
    boardTheme: pick(random, ['brown', 'green', 'blue']),
    lightTheme: 'kchess',
    darkTheme: pick(random, ['kchess', 'ocean']),
    pieceSet: 'cburnett',
    pieceAnimation: pick(random, ['none', 'fast', 'normal', 'slow', 'warp']),
    coordinates: pick(random, ['inside', 'outside', 'none', 'bogus']),
    soundEnabled: random() < 0.5,
    soundVolume: pick(random, [0.25, 0.7, 1, 80]),
    enginePath: pick(random, ['', '/usr/bin/stockfish']),
    premove: random() < 0.5,
    promotion: pick(random, ['ask', 'queen', 'premove', 'rook']),
    showLegalMoves: random() < 0.5,
    notificationsEnabled: random() < 0.5,
    notifyActive: random() < 0.5,
    notifyBackground: random() < 0.5,
    notifyOpponentMove: random() < 0.5,
    notifyLowTime: random() < 0.5,
    notifyGameEvents: random() < 0.5,
    notifyComputerMove: random() < 0.5,
    notifySound: random() < 0.5,
    voicePushToTalk: random() < 0.5,
    voiceConfirmMoves: random() < 0.5,
    voiceHistory: random() < 0.5,
    updateAutoCheck: random() < 0.5,
    updateAutoDownload: random() < 0.5,
    updateInstallOnQuit: random() < 0.5,
    engineLevels: pick(random, [['club', 'max'], ['max', 'beginner', 'nonsense'], ['gm']]),
    reviewAuto: pick(random, ['off', 'recent', 'all', 'sometimes']),
    reviewOnBattery: random() < 0.5,
    receiveChallenges: random() < 0.5,
    notifyChallenges: random() < 0.5,
    onlineChat: random() < 0.5,
    correspondencePoll: pick(random, [0, 5, 300]),
    zenMode: random() < 0.5,
    blindfold: random() < 0.5,
    cloudEval: random() < 0.5,
    showOpeningName: random() < 0.5,
    swipeNavigation: random() < 0.5,
    swipeIndicator: random() < 0.5,
  }
}

function randomQuery(random: Random): Record<string, unknown> {
  const query: Record<string, unknown> = {
    offset: random() < 0.2 ? 0 : int(random, 0, 60),
    limit: random() < 0.05 ? 101 : int(random, 1, 25),
  }
  if (random() < 0.5) query.account = pick(random, ['alice', 'BOB', 'carol', 'nobody'])
  if (random() < 0.4) query.result = pick(random, ['win', 'loss', 'draw'])
  if (random() < 0.3) query.rated = random() < 0.5
  if (random() < 0.03) query.result = 'other'
  return query
}

interface Operation {
  name: string
  args: unknown[]
  run: () => unknown
}

function randomOperation(random: Random): Operation {
  const account = pick(random, ACCOUNTS)
  const games = Array.from({ length: int(random, 0, 6) }, () => randomGame(random))
  const current = random() < 0.9
  const weights: [string, () => Operation][] = [
    [
      'addAccount',
      () => {
        const connected = random() < 0.5
        return {
          name: 'addAccount',
          args: [account, connected],
          run: () => store.addAccount(account, connected),
        }
      },
    ],
    [
      'addFriends',
      () => {
        const names = Array.from({ length: int(random, 0, 3) }, () => pick(random, ACCOUNTS))
        return { name: 'addFriends', args: [names], run: () => store.addFriends(names) }
      },
    ],
    [
      'removeAccount',
      () => ({ name: 'removeAccount', args: [account], run: () => store.removeAccount(account) }),
    ],
    [
      'logoutAccounts',
      () => {
        const scope = random() < 0.5 ? undefined : account
        return {
          name: 'logoutAccounts',
          args: [scope ?? null],
          run: () => store.logoutAccounts(scope),
        }
      },
    ],
    [
      'clearAccountData',
      () => ({
        name: 'clearAccountData',
        args: [account],
        run: () => store.clearAccountData(account),
      }),
    ],
    [
      'saveSettings',
      () => {
        const settings = randomSettings(random) as never
        return { name: 'saveSettings', args: [settings], run: () => store.saveSettings(settings) }
      },
    ],
    [
      'saveGames',
      () => {
        const synced = int(random, 1, 90) * 86_400_000
        return {
          name: 'saveGames',
          args: [account, games, synced, current],
          run: () => store.saveGames(account, games as never, synced, () => current),
        }
      },
    ],
    [
      'saveGamesPage',
      () => ({
        name: 'saveGamesPage',
        args: [account, games, [], current],
        run: () => store.saveGamesPage(account, games as never, [], () => current),
      }),
    ],
    [
      'writeApiCache',
      () => {
        const key = pick(random, API_KEYS)
        const value = random() < 0.5 ? { games: games.length, ok: true } : ['x', 'y']
        return {
          name: 'writeApiCache',
          args: [key, value, current],
          run: () => store.writeApiCache(key, value, () => current),
        }
      },
    ],
    [
      'readApiCache',
      () => {
        const key = pick(random, API_KEYS)
        return { name: 'readApiCache', args: [key], run: () => store.readApiCache(key) }
      },
    ],
    [
      'gamePage',
      () => {
        const query = randomQuery(random)
        return { name: 'gamePage', args: [query], run: () => store.gamePage(query as never) }
      },
    ],
    [
      'gameLibraryOverview',
      () => ({ name: 'gameLibraryOverview', args: [], run: () => store.gameLibraryOverview() }),
    ],
    [
      'gameRatingHistory',
      () => ({
        name: 'gameRatingHistory',
        args: [account],
        run: () => store.gameRatingHistory(account),
      }),
    ],
    [
      'pendingGameIds',
      () => ({ name: 'pendingGameIds', args: [account], run: () => store.pendingGameIds(account) }),
    ],
    [
      'gamePgn',
      () => {
        const id = `G${String(int(random, 0, 40)).padStart(4, '0')}`
        return { name: 'gamePgn', args: [account, id], run: () => store.gamePgn(account, id) }
      },
    ],
    ['getSettings', () => ({ name: 'getSettings', args: [], run: () => store.getSettings() })],
    ['loadData', () => ({ name: 'loadData', args: [], run: () => store.loadData() })],
    [
      'dismissedFriends',
      () => ({ name: 'dismissedFriends', args: [], run: () => store.dismissedFriends() }),
    ],
    ['resetStore', () => ({ name: 'resetStore', args: [], run: () => store.resetStore() })],
  ]
  // Writes dominate, as they do in a sync; each factory draws its own inputs from `random`.
  const writes = [6, 7, 8, 8, 2, 3].map((index) => weights[index]!)
  const reads = [9, 10, 11, 12, 13, 14, 15, 16, 17, 18].map((index) => weights[index]!)
  const choices = [...weights.slice(0, 6), ...writes, ...reads, ...writes]
  return pick(random, choices)[1]()
}

const SEEDS = [11, 2024, 90210]
const OPERATIONS_PER_SEED = 110
const DATA = 'kchess.db'

/** Calls compared per function: successful results and failing ones, across every seed. */
const tally = new Map<string, { ok: number; error: Set<string> }>()

function record(name: string, outcome: Outcome): void {
  const entry = tally.get(name) ?? { ok: 0, error: new Set<string>() }
  if ('ok' in outcome) entry.ok += 1
  else entry.error.add(outcome.error)
  tally.set(name, entry)
}

describe('Rust store.games against the TypeScript store', () => {
  for (const seed of SEEDS) {
    it(`matches every call and table after each step (seed ${seed}, ${OPERATIONS_PER_SEED} steps)`, async () => {
      const random = rng(seed)
      const tsDir = mkdtempSync(join(root, 'ts-'))
      const rustDir = mkdtempSync(join(root, 'rust-'))
      closeDb()
      useTestPlatform({ dataDir: tsDir, secrets: fakeSecrets(false) })
      // Edge cases every seed reaches before the random sequence: a broken row (rejected by the
      // table, rolling back the whole page), a cancelled sync, a live game kept as pending, and
      // a removed account rejecting its sync.
      const page = [
        {
          id: 'Live0001',
          account: 'Alice',
          status: 'started',
          color: 'white',
          opponent: 'Bob',
          moves: '',
          createdAt: 1,
          lastMoveAt: 1,
          rated: true,
          speed: 'blitz',
          perf: 'blitz',
        },
        // No creation time: the table's NOT NULL constraint rejects the whole page.
        {
          id: 'Broken01',
          account: 'Alice',
          status: 'mate',
          color: 'white',
          opponent: 'Bob',
          moves: '',
          lastMoveAt: 1,
          rated: true,
          speed: 'blitz',
          perf: 'blitz',
        },
      ]
      const prelude: Operation[] = [
        { name: 'addAccount', args: ['Alice', true], run: () => store.addAccount('Alice', true) },
        {
          name: 'saveGamesPage',
          args: ['Alice', page, [], true],
          run: () => store.saveGamesPage('Alice', page as never, [], () => true),
        },
        {
          name: 'saveGames',
          args: ['Alice', [], 500, false],
          run: () => store.saveGames('Alice', [], 500, () => false),
        },
        {
          name: 'saveGames',
          args: ['Ghost', [], 500, true],
          run: () => store.saveGames('Ghost', [], 500, () => true),
        },
      ]
      try {
        for (const operation of prelude) {
          const expected = await tsOutcome(operation.run)
          const actual = await rustOutcome(rustDir, `store.games.${operation.name}`, operation.args)
          expect(maskTimes(actual), `${operation.name} (edge case)`).toEqual(maskTimes(expected))
          expect(dumpRust(rustDir), `tables after ${operation.name} (edge case)`).toEqual(dumpTs())
          record(operation.name, expected)
        }
        for (let step = 0; step < OPERATIONS_PER_SEED; step++) {
          const operation = randomOperation(random)
          const expected = await tsOutcome(operation.run)
          const actual = await rustOutcome(rustDir, `store.games.${operation.name}`, operation.args)
          expect(maskTimes(actual), `${operation.name} at step ${step}`).toEqual(
            maskTimes(expected),
          )
          record(operation.name, expected)
          expect(dumpRust(rustDir), `tables after ${operation.name} at step ${step}`).toEqual(
            dumpTs(),
          )
        }
      } finally {
        closeDb()
        rmSync(tsDir, { recursive: true, force: true })
        rmSync(rustDir, { recursive: true, force: true })
      }
    }, 120_000)
  }

  it('exercises every function with successful and failing calls', () => {
    const names = [
      'addAccount',
      'addFriends',
      'removeAccount',
      'logoutAccounts',
      'clearAccountData',
      'saveSettings',
      'saveGames',
      'saveGamesPage',
      'writeApiCache',
      'readApiCache',
      'gamePage',
      'gameLibraryOverview',
      'gameRatingHistory',
      'pendingGameIds',
      'gamePgn',
      'getSettings',
      'loadData',
      'dismissedFriends',
      'resetStore',
    ]
    for (const name of names) {
      expect(tally.get(name)?.ok ?? 0, `${name} successful calls`).toBeGreaterThan(0)
    }
    // Failing paths that matter for parity must also have been reached.
    const failures = new Set([...tally.values()].flatMap((entry) => [...entry.error]))
    expect([...failures].some((message) => message.includes('Sync was cancelled'))).toBe(true)
    expect([...failures].some((message) => message.includes('removed during sync'))).toBe(true)
    expect([...failures].some((message) => /NOT NULL constraint/.test(message))).toBe(true)
  })
})

/** Rows an earlier release would have written, inserted only into tables and columns that exist. */
function seedHistory(db: DatabaseSync): void {
  const insert = (table: string, row: Record<string, string | number>): void => {
    const columns = new Set(
      (db.prepare(`PRAGMA table_info("${table}")`).all() as { name: string }[]).map(
        (column) => column.name,
      ),
    )
    const names = Object.keys(row).filter((name) => columns.has(name))
    if (names.length === 0) return
    db.prepare(
      `INSERT INTO "${table}" (${names.map((name) => `"${name}"`).join(', ')}) VALUES (${names.map(() => '?').join(', ')})`,
    ).run(...names.map((name) => row[name]!))
  }
  insert('settings', {
    id: 1,
    appearance: 'dark',
    boardTheme: 'brown',
    coordinates: 'outside',
    soundEnabled: 0,
    soundVolume: 0.5,
    enginePath: '/bin/engine',
    notifyWhen: 'always',
    colorTheme: 'green',
    engineLevels: 'club,max',
    voiceHistory: 0,
    reviewAuto: 'all',
    reviewOnBattery: 1,
    premove: 0,
    promotion: 'queen',
    pieceAnimation: 'warp',
    correspondencePoll: 7,
    swipeIndicator: 0,
  })
  insert('accounts', { username: 'Alice', connected: 1, lastSyncedAt: 1000 })
  insert('accounts', { username: 'bob', connected: 0 })
  insert('games', {
    account: 'Alice',
    id: 'Done0001',
    createdAt: 1000,
    lastMoveAt: 1000,
    rated: 1,
    speed: 'blitz',
    perf: 'blitz',
    status: 'mate',
    winner: 'white',
    color: 'white',
    opponent: 'Bob',
    opponentRating: 1400,
    playerRating: 1500,
    ratingDiff: 8,
    opening: 'Italian Game',
    moves: 'e4 e5',
    pgn: '1. e4 e5 *',
  })
  insert('games', {
    account: 'Alice',
    id: 'Live0001',
    createdAt: 2000,
    lastMoveAt: 2000,
    rated: 1,
    speed: 'rapid',
    perf: 'rapid',
    status: 'started',
    color: 'black',
    opponent: 'Carol',
    moves: 'd4',
  })
  insert('games', {
    account: 'bob',
    id: 'Done0002',
    createdAt: 3000,
    lastMoveAt: 3000,
    rated: 0,
    speed: 'bullet',
    perf: 'bullet',
    status: 'resign',
    winner: 'black',
    color: 'white',
    opponent: 'Alice',
    moves: 'f4',
  })
  insert('tokens', { username: 'Alice', encrypted: 'c2VjcmV0' })
  insert('api_cache', { key: 'alice:profile', value: '{}', fetchedAt: 5 })
  insert('usage', { account: 'Alice', kind: 'games', requests: 2, bytesIn: 100, since: 0 })
  insert('dismissed_friends', { username: 'Dave', dismissedAt: 3 })
  insert('puzzles', {
    id: 'p1',
    fen: 'k7/8/8/8/8/8/8/K7 w - - 0 1',
    moves: 'a1b1',
    rating: 1500,
    themes: 'mate',
  })
  insert('puzzle_meta', { id: 1, importedAt: 1, count: 1 })
  insert('runs', { kind: 'rush', variant: '', score: 3, detail: '{}', playedAt: 1 })
  insert('voice_log', { at: 1, source: 'voice', heard: 'e4', confidence: 0.9, outcome: 'move' })
  insert('reviews', {
    key: 'k1',
    gameId: 'Done0001',
    source: 'local',
    complete: 1,
    depth: 12,
    data: '{}',
    summary: '{}',
    updatedAt: 1,
  })
  insert('lichess_review_checks', { id: 'Done0001', checkedAt: 1 })
  insert('position_lookups', { key: 'player:{"player":"Alice"}', data: '{}', fetchedAt: 1 })
  insert('documents', { key: 'study:1', body: '{}', updatedAt: 1 })
  insert('archived_games', { id: 'a1', body: '{}', seq: 1 })
}

/** A database as the release that wrote migration `steps` - 1 left it, with rows in it. */
function historicalDatabase(path: string, steps: number): void {
  const db = new DatabaseSync(path)
  try {
    db.exec('PRAGMA foreign_keys = ON')
    for (let version = 0; version < steps; version++) {
      db.exec('BEGIN IMMEDIATE')
      db.exec(MIGRATIONS[version]!)
      db.exec(`PRAGMA user_version = ${version + 1}`)
      db.exec('COMMIT')
    }
    if (steps > 0) seedHistory(db)
  } finally {
    db.close()
  }
}

describe('Rust migrations against the TypeScript migrations', () => {
  for (let steps = 0; steps <= MIGRATIONS.length; steps++) {
    it(`migrates a database from step ${steps} to the same schema and rows`, async () => {
      const base = mkdtempSync(join(root, `history-${steps}-`))
      const tsDir = join(base, 'ts')
      const rustDir = join(base, 'rust')
      mkdirSync(tsDir)
      mkdirSync(rustDir)
      try {
        historicalDatabase(join(base, DATA), steps)
        copyFileSync(join(base, DATA), join(tsDir, DATA))
        copyFileSync(join(base, DATA), join(rustDir, DATA))

        const ts = new DatabaseSync(join(tsDir, DATA))
        ts.exec('PRAGMA foreign_keys = ON')
        try {
          migrate(ts)
          const expected = dumpDatabase(ts)
          expect(expected.userVersion).toBe(MIGRATIONS.length)
          // The Rust core migrates when it opens the database; a read is the first call.
          const core = openRust(rustDir)
          core.callSync('store.games.getSettings', '[]')
          await core.close()
          expect(dumpRust(rustDir)).toEqual(expected)
        } finally {
          ts.close()
        }
      } finally {
        rmSync(base, { recursive: true, force: true })
      }
    })
  }
})
