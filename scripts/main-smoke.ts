import { mergeRatingHistories, ratingHistoryFromGames } from '../app/utils/ratings.ts'
import { isGameInProgress } from '../src/shared/gameStatus.ts'
import { pickConnectedAccount } from '../src/shared/accounts.ts'
import { LichessError, lichessError, throwLichessErrors } from '../src/shared/lichessError.ts'
import { readLines } from '../src/main/ndjson.ts'
import {
  canBoardSeek,
  canDirectChallenge,
  perfFor,
  totalSeconds,
} from '../src/shared/timeControl.ts'
import {
  assertAction,
  assertGameId,
  assertLevel,
  assertMoves,
  assertOnlineOptions,
  assertSettings,
  assertUci,
  assertUsername,
} from '../src/shared/validate.ts'
import { createServer } from 'node:http'
import type { AddressInfo } from 'node:net'
import { DatabaseSync } from 'node:sqlite'
import createClient from 'openapi-fetch'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { mkdirSync, mkdtempSync, readFileSync, statSync, unlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  deleteManagedEngine,
  installManagedEngine,
  managedEngine,
} from '../src/main/managedEngine.ts'
import { MIGRATIONS, migrate } from '../src/main/migrations.ts'
import { pickMacAsset } from '../src/main/stockfishAsset.ts'

const assert = (label: string, actual: unknown, expected: unknown): void => {
  const ok = JSON.stringify(actual) === JSON.stringify(expected)
  console.log(
    `${ok ? 'ok  ' : 'FAIL'} ${label}: ${JSON.stringify(actual)}${ok ? '' : ` (expected ${JSON.stringify(expected)})`}`,
  )
  if (!ok) process.exitCode = 1
}
const throws = (label: string, fn: () => unknown): void => {
  let threw = false
  try {
    fn()
  } catch {
    threw = true
  }
  assert(label, threw, true)
}

// Time controls follow lila's estimated-length formula and Board API limits.
assert('total seconds', totalSeconds(3, 2), 260)
assert('perf: 3+0 is blitz', perfFor(3, 0), 'Blitz')
assert('perf: 15+10 is rapid', perfFor(15, 10), 'Rapid')
assert('perf: 1+0 is bullet', perfFor(1, 0), 'Bullet')
assert('seek needs rapid', [canBoardSeek(5, 0), canBoardSeek(10, 0)], [false, true])
assert(
  'direct challenge allows blitz',
  [canDirectChallenge(2, 0), canDirectChallenge(3, 0)],
  [false, true],
)

// Lichess errors never leak HTML pages.
assert(
  'html 404 becomes not found',
  lichessError({ status: 404 }, '<html>…</html>', 'GET /x').message,
  'Lichess 404 from GET /x: not found',
)
assert(
  'json error detail kept',
  lichessError({ status: 400 }, { error: 'bad' }, 'POST /y') instanceof LichessError,
  true,
)

// IPC validation.
assert('username trimmed', assertUsername('  magnus '), 'magnus')
throws('username rejects path characters', () => assertUsername('../etc'))
throws('username rejects non-strings', () => assertUsername(42))
assert('uci accepts promotion', assertUci('a7a8q'), 'a7a8q')
throws('uci rejects newline injection', () => assertUci('e2e4\nquit'))
throws('moves reject injection in list', () => assertMoves(['e2e4', 'go infinite\nquit']))
throws('moves reject non-array', () => assertMoves('e2e4'))
assert('game id', assertGameId('AbCd1234'), 'AbCd1234')
throws('game id rejects slashes', () => assertGameId('abc/../../x'))
assert('level', assertLevel('high'), 'high')
throws('level rejects unknown', () => assertLevel('insane'))
throws('action rejects unknown', () => assertAction('nuke'))
assert(
  'online options',
  assertOnlineOptions({ minutes: 10, increment: 5, color: 'white', rated: true, target: ' ' }),
  {
    minutes: 10,
    increment: 5,
    color: 'white',
    rated: true,
    target: undefined,
  },
)
throws('online options reject bad target', () =>
  assertOnlineOptions({ minutes: 10, increment: 0, color: 'white', rated: false, target: 'a b' }),
)
throws('online options require rated', () =>
  assertOnlineOptions({ minutes: 10, increment: 0, color: 'white' }),
)
throws('online options reject huge clock', () =>
  assertOnlineOptions({ minutes: 1e9, increment: 0, color: 'white', rated: false }),
)
const settings = {
  appearance: 'dark',
  boardTheme: 'brown',
  coordinates: 'inside',
  soundEnabled: true,
  soundVolume: 3,
  enginePath: '',
  premove: true,
  promotion: 'ask',
  showLegalMoves: true,
}
assert('settings clamp volume', assertSettings(settings).soundVolume, 1)
throws('settings reject NaN volume', () => assertSettings({ ...settings, soundVolume: NaN }))
throws('settings reject bad promotion', () => assertSettings({ ...settings, promotion: 'rook' }))
throws('settings reject bad theme', () => assertSettings({ ...settings, boardTheme: '../x' }))

// Stockfish's current release only ships a universal macOS build; it must be found.
const universal = { name: 'stockfish-macos-universal.tar.gz', browser_download_url: 'u', size: 1 }
const linux = {
  name: 'stockfish-linux-x86-64-universal.tar.gz',
  browser_download_url: 'l',
  size: 1,
}
const m1 = { name: 'stockfish-macos-m1-apple-silicon.tar', browser_download_url: 'm', size: 1 }
assert(
  'finds universal macOS asset',
  pickMacAsset([linux, universal], 'arm64')?.name,
  universal.name,
)
assert('prefers apple silicon on arm64', pickMacAsset([universal, m1], 'arm64')?.name, m1.name)
assert('uses universal on intel', pickMacAsset([m1, universal], 'x64')?.name, universal.name)
assert('ignores non-macOS assets', pickMacAsset([linux], 'arm64'), undefined)

// Choosing which connected account plays online.
const accounts = [
  { username: 'Alice', connected: true },
  { username: 'watched', connected: false },
  { username: 'Bob', connected: true },
]
assert('defaults to the first connected account', pickConnectedAccount(accounts)?.username, 'Alice')
assert(
  'honours a preferred account, ignoring case',
  pickConnectedAccount(accounts, 'bob')?.username,
  'Bob',
)
assert(
  'a tracked account is never picked',
  pickConnectedAccount(accounts, 'watched')?.username,
  'Alice',
)
assert(
  'falls back when the preferred account is gone',
  pickConnectedAccount(accounts, 'carol')?.username,
  'Alice',
)
assert(
  'no connected accounts',
  pickConnectedAccount([{ username: 'watched', connected: false }]),
  undefined,
)
assert(
  'online options carry the account',
  assertOnlineOptions({
    minutes: 10,
    increment: 0,
    color: 'random',
    rated: false,
    account: ' Bob ',
  }).account,
  'Bob',
)
throws('online options reject a bad account', () =>
  assertOnlineOptions({
    minutes: 10,
    increment: 0,
    color: 'random',
    rated: false,
    account: '../x',
  }),
)

// Rating history rebuilt from synced games: rating after = rating before + change.
const at = (iso: string): number => Date.parse(iso)
const rated = (id: string, when: string, before: number, diff: number, extra = {}) => ({
  id,
  account: 'a',
  createdAt: at(when),
  lastMoveAt: at(when),
  rated: true,
  speed: 'blitz',
  perf: 'blitz',
  status: 'mate',
  color: 'white' as const,
  opponent: 'o',
  moves: '',
  playerRating: before,
  ratingDiff: diff,
  ...extra,
})
const rebuilt = ratingHistoryFromGames([
  rated('3', '2026-02-01T20:00:00Z', 1437, 8), // out of order on purpose
  rated('1', '2026-01-31T09:00:00Z', 1430, 2),
  rated('2', '2026-01-31T22:00:00Z', 1432, 5), // same UTC day as game 1: the later one wins
  rated('4', '2026-02-02T10:00:00Z', 1445, 9, { rated: false }), // casual: ignored
  rated('5', '2026-02-02T10:00:00Z', 1500, 3, { perf: 'bullet' }),
])
assert(
  'one entry per rated perf, named like Lichess',
  rebuilt.map((h) => h.name),
  ['Blitz', 'Bullet'],
)
assert('points are [year, month0, day, rating after], one per day', rebuilt[0]!.points, [
  [2026, 0, 31, 1437],
  [2026, 1, 1, 1445],
])
const official = [
  { name: 'Blitz', points: [[2026, 0, 1, 1400]] },
  { name: 'Rapid', points: [] },
]
const filled = mergeRatingHistories(official, [
  { name: 'Blitz', points: [[2026, 0, 2, 1]] },
  { name: 'Rapid', points: [[2026, 0, 3, 2]] },
])
assert('Lichess history wins where it has points', filled.find((h) => h.name === 'Blitz')?.points, [
  [2026, 0, 1, 1400],
])
assert(
  'games fill perfs Lichess left empty',
  filled.filter((h) => h.name === 'Rapid').at(-1)?.points,
  [[2026, 0, 3, 2]],
)

// A live game before its first move (`created`) must not be treated as over.
assert(
  'in-progress statuses',
  ['created', 'started', undefined].map((status) => isGameInProgress(status)),
  [true, true, true],
)
assert(
  'finished statuses',
  ['mate', 'resign', 'aborted', 'timeout', 'draw', 'outoftime', 'noStart'].map((status) =>
    isGameInProgress(status),
  ),
  [false, false, false, false, false, false, false],
)

// Schema migrations track `PRAGMA user_version`.
const version = (db: DatabaseSync): unknown =>
  (db.prepare('PRAGMA user_version').get() as { user_version: number }).user_version
const fresh = new DatabaseSync(':memory:')
migrate(fresh)
assert('fresh database is at the latest version', version(fresh), MIGRATIONS.length)
migrate(fresh)
assert('migrating twice is a no-op', version(fresh), MIGRATIONS.length)
// A database from before versioning: tables exist, user_version is 0, data must survive.
const old = new DatabaseSync(':memory:')
old.exec(MIGRATIONS[0]!)
old.exec("INSERT INTO accounts (username, connected) VALUES ('magnus', 1)")
assert('pre-versioning database starts at 0', version(old), 0)
migrate(old)
assert('pre-versioning database is upgraded', version(old), MIGRATIONS.length)
assert(
  'pre-versioning data survives',
  (old.prepare('SELECT username FROM accounts').get() as { username: string }).username,
  'magnus',
)

// NDJSON streams: lines split across chunks, keep-alive blanks, and a final line without a newline.
const encoder = new TextEncoder()
const chunks = ['{"a":1}\n{"b"', ':2}\n\n\n', '  \r\n{"c":3}']
const streamed: string[] = []
await readLines(
  new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
      controller.close()
    },
  }),
  (line) => streamed.push(line),
)
assert('ndjson lines', streamed, ['{"a":1}', '{"b":2}', '{"c":3}'])

// The middleware turns non-2xx responses into LichessErrors and leaves successes alone.
const server = createServer((request, response) => {
  if (request.url === '/ok')
    response.writeHead(200, { 'Content-Type': 'application/json' }).end('{"n":1}')
  else if (request.url === '/missing')
    response.writeHead(404, { 'Content-Type': 'text/html' }).end('<html>Not found</html>')
  else response.writeHead(400, { 'Content-Type': 'application/json' }).end('{"error":"bad seek"}')
})
await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
const api = createClient<Record<string, never>>({
  baseUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}`,
})
api.use(throwLichessErrors)
const call = (path: string): Promise<unknown> =>
  (api as unknown as { GET: (p: string) => Promise<{ data: unknown }> }).GET(path)
assert('middleware passes success through', ((await call('/ok')) as { data: unknown }).data, {
  n: 1,
})
const failure = async (path: string): Promise<unknown> => {
  try {
    await call(path)
    return 'no error'
  } catch (cause) {
    return cause instanceof LichessError
      ? [cause.status, cause.endpoint, cause.message]
      : String(cause)
  }
}
assert('middleware hides html 404 bodies', await failure('/missing'), [
  404,
  'GET /missing',
  'Lichess 404 from GET /missing: not found',
])
assert('middleware keeps json error detail', await failure('/bad'), [
  400,
  'GET /bad',
  'Lichess 400 from GET /bad: {"error":"bad seek"}',
])
server.close()

// Downloaded engine: install, verify, report, delete — against a fake GitHub release on a temp dir.
const work = mkdtempSync(join(tmpdir(), 'kchess-managed-test-'))
mkdirSync(join(work, 'pkg/stockfish-macos-universal'), { recursive: true })
writeFileSync(
  join(work, 'pkg/stockfish-macos-universal/stockfish-macos-universal'),
  '#!/bin/sh\necho uciok\n',
)
execFileSync('tar', ['-cf', join(work, 'asset.tar'), '-C', join(work, 'pkg'), '.'])
const tarball = readFileSync(join(work, 'asset.tar'))
const digest = (bytes: Buffer): string =>
  `sha256:${createHash('sha256').update(bytes).digest('hex')}`
let releaseDigest: string | null = digest(tarball)
let releaseTag = 'sf_test'
let assetDownloads = 0
const engineServer = createServer((request, response) => {
  const base = `http://127.0.0.1:${(engineServer.address() as AddressInfo).port}`
  if (request.url === '/release')
    response.writeHead(200, { 'Content-Type': 'application/json' }).end(
      JSON.stringify({
        tag_name: releaseTag,
        assets: [
          {
            name: 'stockfish-macos-universal.tar',
            browser_download_url: `${base}/asset.tar`,
            size: tarball.length,
            digest: releaseDigest,
          },
        ],
      }),
    )
  else {
    assetDownloads++
    response.writeHead(200).end(tarball)
  }
})
await new Promise<void>((resolve) => engineServer.listen(0, '127.0.0.1', resolve))
const releaseUrl = `http://127.0.0.1:${(engineServer.address() as AddressInfo).port}/release`
const engineDir = join(work, 'engine')
assert('nothing installed at first', (await managedEngine(engineDir)).installed, false)
const installed = await installManagedEngine({ dir: engineDir, releaseUrl })
assert('install reports the release tag', installed.version, 'sf_test')
assert('installed engine is found', (await managedEngine(engineDir)).version, 'sf_test')
assert('first install downloads', [installed.updated, assetDownloads], [true, 1])
const again = await installManagedEngine({ dir: engineDir, releaseUrl })
assert('already on latest: reports up to date', [again.updated, again.version], [false, 'sf_test'])
assert('already on latest: downloads nothing', assetDownloads, 1)
releaseTag = 'sf_next'
const upgraded = await installManagedEngine({ dir: engineDir, releaseUrl })
assert(
  'newer release is downloaded',
  [upgraded.updated, upgraded.version, assetDownloads],
  [true, 'sf_next', 2],
)
assert('newer release is recorded', (await managedEngine(engineDir)).version, 'sf_next')
// An install from before versions were recorded has no VERSION file and must update once.
unlinkSync(join(engineDir, 'VERSION'))
const legacy = await installManagedEngine({ dir: engineDir, releaseUrl })
assert('unversioned install is refreshed', [legacy.updated, assetDownloads], [true, 3])
assert(
  'installed engine is executable',
  (statSync(join(engineDir, 'stockfish')).mode & 0o111) !== 0,
  true,
)
releaseDigest = digest(Buffer.from('tampered'))
const rejected = await installManagedEngine({ dir: join(work, 'engine2'), releaseUrl }).then(
  () => 'installed',
  (cause: Error) => cause.message,
)
assert('wrong digest is rejected', rejected, 'Stockfish download failed its SHA-256 check.')
assert('rejected download installs nothing', existsSync(join(work, 'engine2')), false)
releaseDigest = null
const unverifiable = await installManagedEngine({ dir: join(work, 'engine3'), releaseUrl }).then(
  () => 'installed',
  (cause: Error) => cause.message,
)
assert(
  'release without digest is rejected',
  unverifiable,
  'The Stockfish release has no SHA-256 digest, so it cannot be verified.',
)
await deleteManagedEngine(engineDir)
assert('delete removes the downloaded engine', (await managedEngine(engineDir)).installed, false)
await deleteManagedEngine(engineDir)
assert('deleting twice is harmless', existsSync(engineDir), false)
engineServer.close()
