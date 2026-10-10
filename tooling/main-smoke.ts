import { mergeRatingHistories, ratingHistoryFromGames } from '../crates/kchess-wasm/js/ratings.ts'
import { isGameInProgress } from '../crates/kchess-wasm/js/gameStatus.ts'
import { pickConnectedAccount } from '../crates/kchess-wasm/js/accounts.ts'
import { LichessError, lichessError } from '../crates/kchess-wasm/js/lichessError.ts'
import {
  canBoardSeek,
  canDirectChallenge,
  perfFor,
  totalSeconds,
} from '../crates/kchess-wasm/js/timeControl.ts'
import {
  assertAction,
  assertGameId,
  assertLevel,
  assertMoves,
  assertNotification,
  assertOnlineOptions,
  assertReviewKey,
  assertReviewRequest,
  assertSettings,
  assertTheme,
  assertUci,
  assertUsername,
} from '../crates/kchess-wasm/js/validate.ts'
import { createServer } from 'node:http'
import type { AddressInfo } from 'node:net'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { nativeCall } from '../crates/kchess-node/js/nativeCore.ts'
import { pickMacAsset } from '../crates/kchess-node/js/stockfishAsset.ts'
import { setPlatform } from '../crates/kchess-node/js/platform.ts'
import { installNativeRules } from './native-rules.ts'

installNativeRules()

// The managed engine's test locations (a directory and a release endpoint) reach the Rust core's
// `managedEngine.*` methods directly; the core honours them only with its test switch on.
interface TestLocation {
  dir: string
  releaseUrl?: string
}
let installRequests = 0
const managedEngine = (location: TestLocation) =>
  nativeCall<{ installed: boolean; version?: string }>('managedEngine.status', location)
const installManagedEngine = (location: TestLocation) =>
  nativeCall<{ path: string; version: string; updated: boolean }>(
    'managedEngine.install',
    ++installRequests,
    location,
  )
const deleteManagedEngine = (location: TestLocation) =>
  nativeCall<unknown>('managedEngine.delete', ++installRequests, location)
// The engine calls run through the Rust core: it needs a platform, and the test-only directory
// and release overrides of the managed engine need the test switch. Handoff refusals are covered
// by the Rust tests.
process.env.KCHESS_STORE_DEBUG = '1'
setPlatform({
  dataDir: mkdtempSync(join(tmpdir(), 'kchess-smoke-')),
  bundledEnginePath: '',
  secrets: {
    available: () => false,
    encrypt: (plain: string) => plain,
    decrypt: (text: string) => text,
  },
  openExternal: async () => {},
  onBattery: () => false,
})

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
assert('level', assertLevel('max'), 'max')
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
  lightTheme: 'kchess',
  darkTheme: 'nord',
  pieceSet: 'merida',
  pieceAnimation: 'fast',
  coordinates: 'inside',
  soundEnabled: true,
  soundVolume: 3,
  enginePath: '',
  premove: true,
  promotion: 'ask',
  showLegalMoves: true,
  notificationsEnabled: true,
  notifyActive: false,
  notifyBackground: true,
  notifyOpponentMove: true,
  notifyLowTime: true,
  notifyGameEvents: true,
  notifyComputerMove: false,
  notifySound: false,
  voicePushToTalk: false,
  voiceConfirmMoves: true,
  voiceHistory: true,
  updateAutoCheck: true,
  updateAutoDownload: true,
  updateInstallOnQuit: true,
  engineLevels: ['max', 'beginner'],
  reviewAuto: 'recent',
  reviewOnBattery: false,
  receiveChallenges: true,
  notifyChallenges: true,
  onlineChat: true,
  correspondencePoll: 5,
  zenMode: false,
  blindfold: false,
  cloudEval: false,
  showOpeningName: true,
  swipeNavigation: true,
  swipeIndicator: true,
}
assert('settings clamp volume', assertSettings(settings).soundVolume, 1)
throws('settings reject bad review mode', () =>
  assertSettings({ ...settings, reviewAuto: 'sometimes' }),
)
const startFen = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
assert(
  'review request accepted',
  assertReviewRequest({ fen: startFen, moves: ['e2e4'], gameId: 'abcdefgh' }).moves,
  ['e2e4'],
)
throws('review request rejects injected moves', () =>
  assertReviewRequest({ fen: startFen, moves: ['e2e4\nquit'] }),
)
throws('review request rejects injected FEN', () =>
  assertReviewRequest({ fen: `${startFen}\ngo infinite`, moves: [] }),
)
throws('review key must be a hash', () => assertReviewKey('../../etc'))
throws('settings reject non-boolean update preference', () =>
  assertSettings({ ...settings, updateAutoCheck: 'yes' }),
)
throws('settings reject NaN volume', () => assertSettings({ ...settings, soundVolume: NaN }))
assert('settings order levels', assertSettings(settings).engineLevels, ['beginner', 'max'])
throws('settings keep a level', () => assertSettings({ ...settings, engineLevels: [] }))
throws('settings reject bad promotion', () => assertSettings({ ...settings, promotion: 'rook' }))
throws('settings reject bad theme id', () => assertSettings({ ...settings, darkTheme: 'No Way!' }))
assert(
  'custom theme accepted',
  assertTheme({ id: 'mine', name: ' Mine ', dark: { bg: '#111', text: '#eee', primary: '#0af' } })
    .name,
  'Mine',
)
throws('custom theme rejects non-hex colors', () =>
  assertTheme({ id: 'x', name: 'X', dark: { bg: 'red', text: '#eee', primary: '#0af' } }),
)
throws('custom theme needs a palette', () => assertTheme({ id: 'x', name: 'X' }))
throws('settings reject bad piece set', () => assertSettings({ ...settings, pieceSet: '../x' }))
throws('settings reject bad animation', () =>
  assertSettings({ ...settings, pieceAnimation: 'warp' }),
)
throws('settings reject non-boolean notify switch', () =>
  assertSettings({ ...settings, notifyActive: 'yes' }),
)
throws('settings require every notification switch', () => {
  const { notifyLowTime: _omitted, ...rest } = settings
  return assertSettings(rest)
})
assert(
  'notification request accepted',
  assertNotification({ kind: 'lowTime', title: 'Low on time', body: '0:20 left' }).kind,
  'lowTime',
)
throws('notification rejects unknown kind', () =>
  assertNotification({ kind: 'spam', title: 't', body: 'b' }),
)
throws('notification rejects huge body', () =>
  assertNotification({ kind: 'test', title: 't', body: 'x'.repeat(401) }),
)
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
  'one entry per rated perf, named by perf key',
  rebuilt.map((h) => h.name),
  ['blitz', 'bullet'],
)
assert('points are [year, month0, day, rating after], one per day', rebuilt[0]!.points, [
  [2026, 0, 31, 1437],
  [2026, 1, 1, 1445],
])
const official = [
  { name: 'blitz', points: [[2026, 0, 1, 1400]] },
  { name: 'rapid', points: [] },
]
const filled = mergeRatingHistories(official, [
  { name: 'blitz', points: [[2026, 0, 2, 1]] },
  { name: 'rapid', points: [[2026, 0, 3, 2]] },
])
assert('Lichess history wins where it has points', filled.find((h) => h.name === 'blitz')?.points, [
  [2026, 0, 1, 1400],
])
assert(
  'games fill perfs Lichess left empty',
  filled.filter((h) => h.name === 'rapid').at(-1)?.points,
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

// Downloaded engine: install, verify, report, delete — against a fake GitHub release on a temp dir.
const work = mkdtempSync(join(tmpdir(), 'kchess-managed-test-'))
// Name the fake build for this host, as the installer only accepts this platform's official asset.
const hostBuild =
  process.platform === 'darwin'
    ? 'stockfish-macos-universal'
    : `stockfish-linux-${process.arch === 'arm64' ? 'arm64' : 'x86-64'}-universal`
const hostAsset = `${hostBuild}${process.platform === 'darwin' ? '.tar' : '.tar.gz'}`
mkdirSync(join(work, `pkg/${hostBuild}`), { recursive: true })
writeFileSync(join(work, `pkg/${hostBuild}/${hostBuild}`), '#!/bin/sh\necho uciok\n')
execFileSync('tar', [
  process.platform === 'darwin' ? '-cf' : '-czf',
  join(work, 'asset.tar'),
  '-C',
  join(work, 'pkg'),
  '.',
])
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
            name: hostAsset,
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
assert('nothing installed at first', (await managedEngine({ dir: engineDir })).installed, false)
const installed = await installManagedEngine({ dir: engineDir, releaseUrl })
assert('install reports the release tag', installed.version, 'sf_test')
assert('installed engine is found', (await managedEngine({ dir: engineDir })).version, 'sf_test')
assert('first install downloads', [installed.updated, assetDownloads], [true, 1])
const again = await installManagedEngine({ dir: engineDir, releaseUrl })
assert('already on latest: reports up to date', [again.updated, again.version], [false, 'sf_test'])
assert('already on latest: downloads nothing', assetDownloads, 1)
releaseTag = 'sf_next'
const location = { dir: engineDir, releaseUrl }
const [upgraded, simultaneous] = await Promise.all([
  installManagedEngine(location),
  installManagedEngine(location),
])
// Exactly one of the simultaneous installs replaces the engine; which one wins is not ordered.
assert(
  'concurrent update downloads and replaces once',
  [[upgraded.updated, simultaneous.updated].filter(Boolean).length, assetDownloads],
  [1, 2],
)
assert('newer release is recorded', (await managedEngine({ dir: engineDir })).version, 'sf_next')
releaseTag = 'sf_next'
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
await deleteManagedEngine({ dir: engineDir })
assert(
  'delete removes the downloaded engine',
  (await managedEngine({ dir: engineDir })).installed,
  false,
)
await deleteManagedEngine({ dir: engineDir })
assert('deleting twice is harmless', existsSync(engineDir), false)
engineServer.close()
