import { describe, it } from 'vitest'
import { rulesLossless } from '../../src/domain/engine.ts'
import { goldenFile } from './golden.ts'

/**
 * The validators and library decoders (`core/src/domain/validate.ts`, `library.ts`) against the
 * Rust port in `crates/kchess-domain/src/misc*`. Each case is a seeded mutation of a sample input,
 * or an edge value; its Rust outcome (value or thrown message) is checked against the golden digest
 * recorded from the TypeScript implementation (see RUST_MIGRATION.md).
 */

const golden = goldenFile('misc')
const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const CASES_PER_SUITE = 90

type Outcome = { ok: unknown } | { error: string }

function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const pick = <T>(random: () => number, items: readonly T[]): T =>
  items[Math.floor(random() * items.length)] as T

/** What a case returns, as JSON: `undefined` becomes null, binary data becomes its summary. */
function normal(value: unknown): unknown {
  if (value === undefined) return null
  return JSON.parse(
    JSON.stringify(value, (_key, inner: unknown) =>
      inner instanceof Uint8Array
        ? { bytes: inner.length, head: Array.from(inner.subarray(0, 4)) }
        : inner,
    ),
  )
}

function outcome(run: () => unknown): Outcome {
  try {
    return { ok: normal(run()) }
  } catch (cause) {
    return { error: cause instanceof Error ? cause.message : String(cause) }
  }
}

/** The export's bytes stay the caller's; the core returns them as null, so compare its summary. */
function exportOutcome(result: Outcome, input: unknown): Outcome {
  if (!('ok' in result) || typeof result.ok !== 'object' || result.ok === null) return result
  const data = (input as { data?: unknown } | null)?.data
  if (!(data instanceof Uint8Array) || (result.ok as { data?: unknown }).data !== null)
    return result
  return {
    ok: {
      ...(result.ok as object),
      data: { bytes: data.length, head: Array.from(data.subarray(0, 4)) },
    },
  }
}

const EDGE_STRINGS = [
  '',
  ' ',
  'x',
  'e2e4',
  'Bob',
  '12345678',
  'ABCDEFGH',
  '#abc',
  '#abcdef',
  '#abcd',
  '1. e4 *',
  START,
  'a'.repeat(1),
  'a'.repeat(8),
  'a'.repeat(12),
  'a'.repeat(13),
  'a'.repeat(30),
  'a'.repeat(31),
  'a'.repeat(40),
  'a'.repeat(41),
  'a'.repeat(60),
  'a'.repeat(61),
  'a'.repeat(80),
  'a'.repeat(81),
  'a'.repeat(100),
  'a'.repeat(101),
  'a'.repeat(120),
  'a'.repeat(140),
  'a'.repeat(141),
  'a'.repeat(200),
  'a'.repeat(201),
  'a'.repeat(2000),
  'a'.repeat(2001),
  'é😀   ​﻿',
  'Ⅻ٣ǅ',
  '\t trimmed \n',
  '2026-03',
  '2026',
  '0123456789abcdef',
]
const EDGE_NUMBERS = [
  0,
  -0,
  1,
  2,
  5,
  50,
  60,
  120,
  179,
  180,
  181,
  365,
  366,
  1000,
  4000,
  4001,
  5000,
  5001,
  1e6,
  1e6 + 1,
  -1,
  0.5,
  1.5,
  0.25,
  9007199254740991,
  9007199254740992,
  1e21,
  1e-7,
  NaN,
  Infinity,
  -Infinity,
]
const EDGE: unknown[] = [
  undefined,
  null,
  true,
  false,
  [],
  [undefined],
  [null],
  {},
  { a: undefined },
  new Uint8Array([71, 73, 70, 56, 57, 97, 1]),
  new Uint8Array([137, 80, 78, 71, 13]),
  new Uint8Array([1, 2]),
  new Uint8Array(0),
  ...EDGE_STRINGS,
  ...EDGE_NUMBERS,
]
const ALPHABET = [
  ...'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789',
  '_',
  '-',
  '.',
  ' ',
  ',',
  "'",
  '&',
  '(',
  ')',
  '#',
  '/',
  '+',
  'é',
  '😀',
  ' ',
  '\t',
  'Ⅻ',
  '́',
]
const EXTRA_KEYS = ['extra', 'version', 'op', 'kind', 'name', 'fen', 'pgn', 'id']

function editString(value: string, random: () => number): string {
  switch (Math.floor(random() * 6)) {
    case 0:
      return value.slice(0, Math.floor(random() * (value.length + 1)))
    case 1:
      return value + pick(random, ALPHABET)
    case 2:
      return pick(random, ALPHABET) + value
    case 3:
      return value.replace(/^./, '') || 'z'
    case 4:
      return value.length ? value.slice(0, -1) + pick(random, ALPHABET) : pick(random, ALPHABET)
    default:
      return value.repeat(2)
  }
}

/** A seeded mutation of a sample: its shape kept mostly, its properties and values changed. */
function mutate(value: unknown, random: () => number, depth = 0): unknown {
  if (depth > 4 || random() < 0.12) return pick(random, EDGE)
  if (value instanceof Uint8Array) return pick(random, EDGE)
  if (Array.isArray(value)) {
    const copy = value.map((item) => mutate(item, random, depth + 1))
    if (random() < 0.3) copy.push(pick(random, EDGE))
    if (random() < 0.2) copy.pop()
    return copy
  }
  if (value !== null && typeof value === 'object') {
    const out: Record<string, unknown> = { ...(value as Record<string, unknown>) }
    const keys = Object.keys(out)
    const key = keys.length ? pick(random, keys) : 'missing'
    const action = random()
    if (action < 0.2 && keys.length) delete out[key]
    else if (action < 0.35 && keys.length) out[key] = undefined
    else if (action < 0.5) out[pick(random, EXTRA_KEYS)] = pick(random, EDGE)
    else if (keys.length) out[key] = mutate(out[key], random, depth + 1)
    return out
  }
  if (typeof value === 'string') return editString(value, random)
  if (typeof value === 'number')
    return random() < 0.5 ? value + pick(random, [-1, 1, 0.5, 1000]) : pick(random, EDGE_NUMBERS)
  if (typeof value === 'boolean') return !value
  return pick(random, EDGE)
}

/** Random JSON-like values, shallow. */
function randomValue(random: () => number, depth: number): unknown {
  const roll = random()
  if (roll < 0.3 || depth > 2) return pick(random, EDGE)
  if (roll < 0.5)
    return Array.from({ length: Math.floor(random() * 4) }, () => randomValue(random, depth + 1))
  if (roll < 0.7) {
    const fields: Record<string, unknown> = {}
    for (let i = 0; i < Math.floor(random() * 5); i++)
      fields[pick(random, ['a', 'b', 'c', 'op', 'account', 'fen', ...EXTRA_KEYS])] = randomValue(
        random,
        depth + 1,
      )
    return fields
  }
  return Array.from({ length: Math.floor(random() * 12) }, () => pick(random, ALPHABET)).join('')
}

/** Cases for a one-argument validator: its samples, their mutations and edge values. */
function singleCases(samples: unknown[], random: () => number): unknown[][] {
  const cases: unknown[][] = samples.map((sample) => [sample])
  for (let i = 0; i < CASES_PER_SUITE; i++) {
    const roll = random()
    const base = pick(random, samples)
    cases.push([
      roll < 0.7 ? mutate(base, random) : roll < 0.85 ? pick(random, EDGE) : randomValue(random, 0),
    ])
  }
  return cases
}

const KINDS = [
  'analysis',
  'local',
  'computer',
  'archive:computer',
  'archive:board',
  'archive:clock',
  'bogus',
  undefined,
]

const NEW_ARENA = {
  name: 'Friday Blitz',
  clockTime: 3,
  clockIncrement: 2,
  minutes: 60,
  waitMinutes: 5,
  startDate: 1_800_000_000_000,
  variant: 'standard',
  rated: true,
  password: 'pw',
  description: 'Have fun',
}
const SETTINGS = {
  appearance: 'system',
  boardTheme: 'classic',
  lightTheme: 'light',
  darkTheme: 'dark',
  pieceSet: 'cburnett',
  pieceAnimation: 'normal',
  coordinates: 'inside',
  soundEnabled: true,
  soundVolume: 0.5,
  enginePath: '',
  premove: true,
  promotion: 'ask',
  showLegalMoves: true,
  notificationsEnabled: true,
  notifyActive: true,
  notifyBackground: false,
  notifyOpponentMove: true,
  notifyLowTime: true,
  notifyGameEvents: true,
  notifyComputerMove: false,
  notifySound: true,
  voicePushToTalk: false,
  voiceConfirmMoves: true,
  voiceHistory: true,
  updateAutoCheck: true,
  updateAutoDownload: false,
  updateInstallOnQuit: true,
  engineLevels: ['club', 'gm'],
  reviewAuto: 'recent',
  reviewOnBattery: false,
  receiveChallenges: true,
  notifyChallenges: true,
  onlineChat: true,
  correspondencePoll: 30,
  zenMode: false,
  blindfold: false,
  cloudEval: false,
  showOpeningName: true,
  swipeNavigation: true,
  swipeIndicator: true,
}
const CORE_SETTINGS = {
  enginePath: '',
  engineLevels: ['club'],
  reviewAuto: 'off',
  reviewOnBattery: false,
  receiveChallenges: true,
  onlineChat: true,
  correspondencePoll: 0,
  cloudEval: false,
  voiceHistory: true,
}
const ONLINE_OPTIONS = {
  minutes: 3,
  increment: 2,
  color: 'random',
  rated: true,
  days: 3,
  variant: 'standard',
  fen: START,
  account: 'abc',
  target: 'bob',
}
const THEME = {
  id: 'midnight',
  name: 'Midnight',
  light: { bg: '#fff', text: '#000', primary: '#123456' },
  dark: { bg: '#000', text: '#fff', primary: '#abc', muted: '#111' },
}
const NOTIFICATION = { kind: 'test', title: 'Hi', body: 'There' }
const PUZZLE_REQUEST = { account: '', angle: 'mateIn2', difficulty: 'normal', color: 'white' }
const PUZZLE_SOLVE = { account: 'abc', angle: 'mateIn2', id: 'abc123', win: true, rated: false }
const LOCAL_QUERY = { theme: 'mateIn2', minRating: 1000, maxRating: 2000, count: 10 }
const LADDER_QUERY = { from: 1000, to: 2000, count: 10 }
const RUN_INPUT = {
  kind: 'storm',
  variant: 'standard',
  score: 12,
  detail: { a: 1, b: 'x', c: true },
}
const BEST_MOVE = { fen: START, movetime: 500, chess960: false }
const ANALYSIS = {
  fen: START,
  lines: 3,
  infinite: false,
  rootFen: START,
  moves: ['e2e4'],
  clientId: 1,
}
const REVIEW = { fen: START, moves: ['e2e4', 'e7e5'], gameId: 'abcdefgh', account: 'abc' }
const VOICE_ATTEMPT = {
  source: 'computer',
  heard: 'e four',
  confidence: 0.9,
  words: [{ word: 'e', conf: 0.9 }],
  outcome: 'played',
  parsed: 'e4',
  expected: 'e4',
  fen: START,
  retryOf: 1,
}
const GAME_PAGE = { account: 'abc', result: 'win', rated: true, offset: 0, limit: 10 }
const STUDY_SYNC = { account: 'abc', studyId: 'abcdefgh', baseline: '1. e4 *', pgn: '1. d4 *' }
const LOOKUP = {
  speeds: ['blitz'],
  ratings: [1600],
  player: 'abc',
  color: 'white',
  modes: ['rated'],
  since: '2020-01',
}
const INSIGHTS = { account: 'abc', speed: 'blitz', rated: true, days: 30 }
const EXPORT_PGN = { name: 'game 1.pgn', kind: 'pgn', data: '1. e4 *' }
const EXPORT_GIF = { name: 'anim', kind: 'gif', data: new Uint8Array([71, 73, 70, 56, 57, 97, 1]) }
const EXPORT_PNG = { name: 'img', kind: 'png', data: new Uint8Array([137, 80, 78, 71, 13]) }
const ANALYSIS_SESSION = {
  version: 1,
  pgn: '1. e4 e5 2. Nf3 *',
  path: 'e2e4 e7e5',
  orientation: 'black',
  study: 's1',
  chapter: 'c1',
}
const LOCAL_SESSION = {
  version: 1,
  setup: { variant: 'standard', fen: START },
  moves: ['e2e4', 'e7e5'],
  clock: { white: { minutes: 3, increment: 2 }, black: { minutes: 3, increment: 0 } },
  times: { white: 1000, black: 2000 },
  result: { winner: 'white', reason: 'resign' },
}
const COMPUTER_SESSION = {
  version: 2,
  moves: ['e2e4'],
  ply: 1,
  level: 'club',
  color: 'white',
  resigned: false,
  setup: { variant: 'chess960', fen: START },
  clock: { minutes: 3, increment: 2 },
  times: { white: 5, black: 6 },
  flagged: 'black',
}
const ARCHIVE_IDENTITY = { id: 'g123', startedAt: 1700 }
const SESSIONS = [ANALYSIS_SESSION, LOCAL_SESSION, COMPUTER_SESSION, ARCHIVE_IDENTITY]
const JOINED = [{ system: 'arena', id: 'abcd1234', account: 'abc', name: 'Friday', until: 1700 }]
const REPERTOIRE = { 'study1:white': { [START]: 2, x: 0, y: 1.5 }, 'study2:black': {} }
const STUDY_COMMANDS = [
  { op: 'save', name: 'S', pgn: '1. e4 *' },
  { op: 'saveChapters', name: 'S', chapters: [{ name: 'c', pgn: '1. e4 *' }] },
  { op: 'addChapter', id: 's1', name: 'c' },
  { op: 'renameChapter', id: 's1', chapterId: 'c1', name: 'n' },
  { op: 'duplicateChapter', id: 's1', chapterId: 'c1' },
  { op: 'removeChapter', id: 's1', chapterId: 'c1' },
  { op: 'markCloud', id: 's1', account: 'abc', remoteId: 'abcdefgh', baseline: '1. e4 *' },
  {
    op: 'offline',
    account: 'abc',
    remote: { id: 'abcdefgh', name: 'R' },
    chapters: [{ name: 'c', pgn: '1. e4 *' }],
  },
  { op: 'remove', id: 's1' },
  { op: 'restore', study: { id: 's' } },
  { op: 'rename', id: 's1', name: 'n' },
  { op: 'duplicate', id: 's1' },
]
const LEGACY = { 'kchess:studies:v1': '{}', 'kchess:tournaments-joined': '[]' }
const ARCHIVED_GAME = {
  id: 'g1',
  source: 'board',
  startedAt: 1,
  updatedAt: 2,
  white: 'A',
  black: 'B',
  result: '*',
  reason: '',
  finished: false,
  setup: { variant: 'standard', fen: START },
  moves: ['e2e4'],
  timeControl: '-',
}

/** Two-argument methods: a kind or a value, each mutated. */
function pairCases(first: unknown[], second: unknown[], random: () => number): unknown[][] {
  const cases: unknown[][] = []
  for (const kind of KINDS) for (const value of second) cases.push([kind, value])
  for (let i = 0; i < CASES_PER_SUITE; i++)
    cases.push([pick(random, [...first, pick(random, KINDS)]), roll(random, second)])
  return cases
}

/** Channels are always a list from the caller's own code (a non-list is a programming error). */
function channelsCases(random: () => number): unknown[][] {
  return pairCases(
    [{ channel: 'player' }, { gameId: 'abcdefgh' }],
    [['player', 'spectator']],
    random,
  ).map(([value]) => [value, ['player', 'spectator']])
}

/** Binary data is not spread into sessions in the tests (the core keeps only its first bytes). */
function roll(random: () => number, samples: unknown[]): unknown {
  const value = random() < 0.7 ? mutate(pick(random, samples), random) : pick(random, EDGE)
  return value instanceof Uint8Array ? null : value
}

function seedOf(name: string): number {
  let seed = 17
  for (const ch of name) seed = (Math.imul(seed, 31) + ch.charCodeAt(0)) | 0
  return seed
}

/** Every validator and decoder: method, and its cases. */
function suites(): Array<{ method: string; cases: unknown[][] }> {
  const random = rng(seedOf('misc-validate'))
  const single: Array<[string, unknown[]]> = [
    ['assertUsername', ['abc', 'Bob_1', 'a-b', 'x'.repeat(30), ' Carol ', 'bad name', 'a']],
    ['assertGameId', ['abcdefgh', 'abcdefghijkl', 'abc', 'abcdefghijklm']],
    ['assertGameIds', [['abcdefgh'], ['abcdefgh', 'zyxwvuts'], []]],
    ['assertUci', ['e2e4', 'a7a8q', 'e2e9', 'e7e8k']],
    ['assertMoves', [['e2e4', 'e7e5'], []]],
    ['assertLevel', ['club', 'gm', 'max', 'nope']],
    ['assertUsernames', [['abc', 'def'], []]],
    ['assertFriendList', [['abc'], []]],
    ['assertAction', ['resign', 'berserk', 'nope']],
    ['assertChatRoom', ['player', 'spectator']],
    ['assertChatText', ['hello', '  hi  ', 'x'.repeat(141), '   ']],
    ['assertNewArena', [NEW_ARENA]],
    ['assertMessageText', ['hi', 'x'.repeat(8001), '  ']],
    ['assertDeclineReason', ['generic', 'variant']],
    ['assertTournamentSystem', ['arena', 'swiss']],
    ['assertTournamentId', ['abcd1234']],
    ['assertTournamentPassword', ['', 'secret', undefined, 'x'.repeat(101)]],
    ['assertExport', [EXPORT_PGN, EXPORT_GIF, EXPORT_PNG]],
    ['assertInsightsQuery', [INSIGHTS]],
    ['assertPerfType', ['blitz', 'crazyhouse']],
    ['assertLichessId', ['abcd1234']],
    ['assertOptionalAccount', ['', 'abc']],
    ['assertOnlineOptions', [ONLINE_OPTIONS]],
    ['assertTheme', [THEME]],
    ['assertNotification', [NOTIFICATION]],
    ['assertCoreSettings', [CORE_SETTINGS]],
    ['assertSettings', [SETTINGS]],
    ['assertPuzzleRequest', [PUZZLE_REQUEST]],
    ['assertPuzzleSolve', [PUZZLE_SOLVE]],
    ['assertDays', [1, 30, 365]],
    ['assertActivityMax', [1, 200]],
    ['assertLocalQuery', [LOCAL_QUERY]],
    ['assertLadderQuery', [LADDER_QUERY]],
    ['assertRunKind', ['storm']],
    ['assertRunInput', [RUN_INPUT]],
    ['assertBestMoveOptions', [BEST_MOVE, undefined]],
    ['assertAnalysisRequest', [ANALYSIS]],
    ['assertReviewRequest', [REVIEW]],
    ['assertReviewKey', ['0123456789abcdef']],
    ['assertVoiceAttempt', [VOICE_ATTEMPT]],
    ['assertVoiceUpdate', [{ outcome: 'correct', expected: 'e4' }]],
    ['assertVoiceId', [1, 5]],
    ['assertVoiceLimit', [1, 5000]],
    ['assertGamePageQuery', [GAME_PAGE]],
    ['assertStudySyncRequest', [STUDY_SYNC]],
    ['assertBroadcastQuery', ['abc', undefined, '  x  ']],
    ['assertLookupOptions', [LOOKUP, undefined]],
    ['decodeAnalysisSession', [ANALYSIS_SESSION]],
    ['decodeLocalSession', [LOCAL_SESSION]],
    ['decodeComputerSession', [COMPUTER_SESSION]],
    ['decodeArchiveIdentity', [ARCHIVE_IDENTITY]],
    ['decodeJoinedTournaments', [JOINED]],
    ['decodeRepertoireMisses', [REPERTOIRE]],
    ['assertStudyCommandShape', STUDY_COMMANDS],
    ['assertArchivedGameShape', [ARCHIVED_GAME]],
    ['assertLibraryId', ['abc']],
    ['assertRepertoireKey', ['x']],
    ['assertSessionKind', ['analysis']],
    ['assertSide', ['white']],
    ['assertLegacyDocuments', [LEGACY]],
    ['isSessionKind', ['analysis', 'bogus']],
  ]
  const result = single.map(([method, samples]) => ({
    method,
    cases: singleCases(samples, rng(seedOf(method))),
  }))
  result.push({
    method: 'assertWatchTarget',
    cases: channelsCases(random),
  })
  for (const method of ['decodeStoredSession', 'encodeSession', 'assertSession']) {
    result.push({
      method,
      cases: pairCases([], SESSIONS, random),
    })
  }
  return result
}

describe('misc rules: validators and library decoders', () => {
  for (const { method, cases } of suites()) {
    it(`${method} matches its golden outcomes`, () => {
      cases.forEach((args, index) => {
        const rust = outcome(() => rulesLossless(method, ...args))
        const fixed = method === 'assertExport' ? exportOutcome(rust, args[0]) : rust
        // The core checks a game's shape and returns the caller's own object, so only acceptance
        // is compared (binary fields inside an echoed object are not summarized by Rust).
        const accepted = (out: Outcome): Outcome => ('ok' in out ? { ok: 'accepted' } : out)
        golden.check(method, index, method === 'assertArchivedGameShape' ? accepted(fixed) : fixed)
      })
    })
  }
})
