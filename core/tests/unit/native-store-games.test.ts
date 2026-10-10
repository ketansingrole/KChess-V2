import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterAll, describe, expect, it } from 'vitest'
import { nativeRules, type NativeCoreHandle } from '../../src/services/native'
import { goldenFile } from './golden'

/**
 * `store.games` (`crates/kchess-core/src/store/games.rs`) and the schema
 * (`crates/kchess-core/src/store/migrations.rs`), run against a native core with random sequences
 * of calls. After each step the result and the contents of every table, the schema and
 * `user_version` are normalised and checked against the golden digests recorded from the
 * TypeScript store before it moved to Rust (`golden/store-games.json`). Timestamps written at the
 * current moment read as `<now>`; everything else must match exactly.
 */

const golden = goldenFile('store-games')
/** The migration SQL of the TypeScript store, in order (test input, not code). */
const MIGRATION_STEPS = (
  JSON.parse(
    readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), 'fixtures', 'store-migrations.json'),
      'utf8',
    ),
  ) as string[]
).length

const root = mkdtempSync(join(tmpdir(), 'kchess-store-parity-'))
afterAll(() => rmSync(root, { recursive: true, force: true }))

const TIME_KEYS = new Set(['fetchedAt', 'dismissedAt', 'updatedAt', 'checkedAt', 'importedAt'])

function messageOf(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause)
}

/** Timestamps written by the core at the current moment read as `<now>`; any other value stays. */
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

type Outcome = { ok: unknown } | { error: string }

function openRust(dataDir: string): NativeCoreHandle {
  const native = nativeRules()
  if (!native) throw new Error('The native rules are not built (pnpm run build:native).')
  return new native.NativeCore(
    { dataDir },
    () => {},
    () => {},
  )
}

/** One call on the core, as a host sees it: the JSON result, or the message it failed with. */
async function rustOutcome(
  core: NativeCoreHandle,
  method: string,
  args: unknown[],
): Promise<Outcome> {
  try {
    return { ok: JSON.parse(core.callSync(method, JSON.stringify(args))) as unknown }
  } catch (cause) {
    return { error: messageOf(cause) }
  }
}

interface TableDump {
  schema: unknown[]
  userVersion: unknown
  tables: Record<string, unknown[]>
}

/** Every schema object, `user_version`, and every table's rows in a total order (the core's snapshot). */
function dumpRust(core: NativeCoreHandle): TableDump {
  const snapshot = JSON.parse(core.callSync('store.debug.tables', '[]')) as TableDump
  return {
    schema: snapshot.schema,
    userVersion: snapshot.userVersion,
    tables: maskTimes(snapshot.tables) as Record<string, unknown[]>,
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
        }
      },
    ],
    [
      'addFriends',
      () => {
        const names = Array.from({ length: int(random, 0, 3) }, () => pick(random, ACCOUNTS))
        return { name: 'addFriends', args: [names] }
      },
    ],
    ['removeAccount', () => ({ name: 'removeAccount', args: [account] })],
    [
      'logoutAccounts',
      () => {
        const scope = random() < 0.5 ? undefined : account
        return {
          name: 'logoutAccounts',
          args: [scope ?? null],
        }
      },
    ],
    [
      'clearAccountData',
      () => ({
        name: 'clearAccountData',
        args: [account],
      }),
    ],
    [
      'saveSettings',
      () => {
        const settings = randomSettings(random) as never
        return { name: 'saveSettings', args: [settings] }
      },
    ],
    [
      'saveGames',
      () => {
        const synced = int(random, 1, 90) * 86_400_000
        return {
          name: 'saveGames',
          args: [account, games, synced, current],
        }
      },
    ],
    [
      'saveGamesPage',
      () => ({
        name: 'saveGamesPage',
        args: [account, games, [], current],
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
        }
      },
    ],
    [
      'readApiCache',
      () => {
        const key = pick(random, API_KEYS)
        return { name: 'readApiCache', args: [key] }
      },
    ],
    [
      'gamePage',
      () => {
        const query = randomQuery(random)
        return { name: 'gamePage', args: [query] }
      },
    ],
    ['gameLibraryOverview', () => ({ name: 'gameLibraryOverview', args: [] })],
    [
      'gameRatingHistory',
      () => ({
        name: 'gameRatingHistory',
        args: [account],
      }),
    ],
    ['pendingGameIds', () => ({ name: 'pendingGameIds', args: [account] })],
    [
      'gamePgn',
      () => {
        const id = `G${String(int(random, 0, 40)).padStart(4, '0')}`
        return { name: 'gamePgn', args: [account, id] }
      },
    ],
    ['getSettings', () => ({ name: 'getSettings', args: [] })],
    ['loadData', () => ({ name: 'loadData', args: [] })],
    ['dismissedFriends', () => ({ name: 'dismissedFriends', args: [] })],
    ['resetStore', () => ({ name: 'resetStore', args: [] })],
  ]
  // Writes dominate, as they do in a sync; each factory draws its own inputs from `random`.
  const writes = [6, 7, 8, 8, 2, 3].map((index) => weights[index]!)
  const reads = [9, 10, 11, 12, 13, 14, 15, 16, 17, 18].map((index) => weights[index]!)
  const choices = [...weights.slice(0, 6), ...writes, ...reads, ...writes]
  return pick(random, choices)[1]()
}

const SEEDS = [11, 2024, 90210]
const OPERATIONS_PER_SEED = 110

/** Calls compared per function: successful results and failing ones, across every seed. */
const tally = new Map<string, { ok: number; error: Set<string> }>()

function record(name: string, outcome: Outcome): void {
  const entry = tally.get(name) ?? { ok: 0, error: new Set<string>() }
  if ('ok' in outcome) entry.ok += 1
  else entry.error.add(outcome.error)
  tally.set(name, entry)
}

describe('Rust store.games', () => {
  for (const seed of SEEDS) {
    it(`matches every call and table after each step (seed ${seed}, ${OPERATIONS_PER_SEED} steps)`, async () => {
      const random = rng(seed)
      const core = openRust(mkdtempSync(join(root, 'rust-')))
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
        { name: 'addAccount', args: ['Alice', true] },
        { name: 'saveGamesPage', args: ['Alice', page, [], true] },
        { name: 'saveGames', args: ['Alice', [], 500, false] },
        { name: 'saveGames', args: ['Ghost', [], 500, true] },
      ]
      try {
        for (const [index, operation] of prelude.entries()) {
          const actual = await rustOutcome(core, `store.games.${operation.name}`, operation.args)
          golden.check('edge cases', index * 2, maskTimes(actual))
          golden.check('edge cases', index * 2 + 1, dumpRust(core))
          record(operation.name, actual)
        }
        for (let step = 0; step < OPERATIONS_PER_SEED; step++) {
          const operation = randomOperation(random)
          const actual = await rustOutcome(core, `store.games.${operation.name}`, operation.args)
          golden.check(`seed ${seed}`, step * 2, maskTimes(actual))
          record(operation.name, actual)
          golden.check(`seed ${seed}`, step * 2 + 1, dumpRust(core))
        }
      } finally {
        await core.close()
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
function seedHistory(): [string, Record<string, string | number>][] {
  const rows: [string, Record<string, string | number>][] = []
  const insert = (table: string, row: Record<string, string | number>): void => {
    rows.push([table, row])
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
  return rows
}

/** A database as the release that applied the first `steps` migrations left it, with rows in it. */
describe('Rust migrations keep the schema and rows of every earlier release', () => {
  for (let steps = 0; steps <= MIGRATION_STEPS; steps++) {
    it(`migrates a database from step ${steps} to the same schema and rows`, async () => {
      const dataDir = mkdtempSync(join(root, `history-${steps}-`))
      // The earlier database is written before the store opens it; the first read migrates it.
      const earlier = openRust(dataDir)
      try {
        earlier.callSync(
          'store.debug.historical',
          JSON.stringify([steps, steps > 0 ? seedHistory() : []]),
        )
      } finally {
        await earlier.close()
      }
      const core = openRust(dataDir)
      try {
        core.callSync('store.games.getSettings', '[]')
        const dump = dumpRust(core)
        expect(dump.userVersion).toBe(MIGRATION_STEPS)
        golden.check(`migration ${steps}`, 0, dump)
      } finally {
        await core.close()
      }
    })
  }
})
