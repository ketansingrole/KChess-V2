import { describe, expect, it } from 'vitest'
import {
  assertAction,
  assertActivityMax,
  assertAnalysisRequest,
  assertBestMoveOptions,
  assertBroadcastQuery,
  assertChatRoom,
  assertChatText,
  assertCoreSettings,
  assertDays,
  assertDeclineReason,
  assertExport,
  assertGameId,
  assertGameIds,
  assertGamePageQuery,
  assertInsightsQuery,
  assertLadderQuery,
  assertLevel,
  assertLichessId,
  assertLocalQuery,
  assertLookupOptions,
  assertMessageText,
  assertMoves,
  assertNewArena,
  assertNotification,
  assertOnlineOptions,
  assertOptionalAccount,
  assertPerfType,
  assertPuzzleRequest,
  assertPuzzleSolve,
  assertReviewKey,
  assertReviewRequest,
  assertRunInput,
  assertRunKind,
  assertSettings,
  assertStudySyncRequest,
  assertTheme,
  assertTournamentId,
  assertTournamentPassword,
  assertTournamentSystem,
  assertUci,
  assertUsernames,
  assertFriendList,
  assertUsername,
  assertVoiceAttempt,
  assertVoiceId,
  assertVoiceLimit,
  assertVoiceUpdate,
  assertWatchTarget,
  ENGINE_LEVELS,
  FEN,
  GAME_ID,
  ONLINE_ACTIONS,
  PUZZLE_ANGLE,
  PUZZLE_ID,
  UCI_MOVE,
  USERNAME,
} from '@kchess/rules/validate'

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
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

/** Input validation at the renderer → main IPC boundary: normalized value or a thrown message. */
describe('names and moves', () => {
  it('trims usernames and rejects bad ones', () => {
    expect(assertUsername('abc')).toBe('abc')
    expect(assertUsername(' Carol ')).toBe('Carol')
    expect(() => assertUsername('a')).toThrow('Enter a valid Lichess username.')
    expect(() => assertUsername('bad name')).toThrow('Enter a valid Lichess username.')
    expect(() => assertUsername(123)).toThrow('Enter a valid Lichess username.')
  })

  it('checks game ids one by one and in lists', () => {
    expect(assertGameId('abcdefgh')).toBe('abcdefgh')
    expect(() => assertGameId('abc')).toThrow('Invalid game id.')
    expect(assertGameIds(['abcdefgh'])).toEqual(['abcdefgh'])
    expect(assertGameIds([])).toEqual([])
    expect(() => assertGameIds(['x'])).toThrow('Invalid game id.')
  })

  it('checks UCI moves, including promotions', () => {
    expect(assertUci('e2e4')).toBe('e2e4')
    expect(assertUci('a7a8q')).toBe('a7a8q')
    expect(assertUci('e7e8b')).toBe('e7e8b')
    expect(() => assertUci('e2e9')).toThrow('Invalid move.')
    expect(() => assertUci('e7e8k')).toThrow('Invalid move.')
    expect(assertMoves(['e2e4', 'e7e5'])).toEqual(['e2e4', 'e7e5'])
    expect(assertMoves([])).toEqual([])
    expect(() => assertMoves(['bogus'])).toThrow('Invalid move.')
  })

  it('checks engine levels, name lists and game actions', () => {
    expect(assertLevel('club')).toBe('club')
    expect(() => assertLevel('nope')).toThrow('Invalid engine level.')
    expect(assertUsernames(['abc', 'def'])).toEqual(['abc', 'def'])
    expect(assertUsernames([])).toEqual([])
    expect(() => assertUsernames(['abc', 'x'])).toThrow('Enter a valid Lichess username.')
    expect(() => assertUsernames('abc')).toThrow('Invalid users.')
    expect(assertFriendList(['abc'])).toEqual(['abc'])
    expect(() => assertFriendList([])).toThrow('Choose at least one player.')
    expect(assertAction('resign')).toBe('resign')
    expect(() => assertAction('nope')).toThrow('Invalid game action.')
  })
})

describe('chat and messaging', () => {
  it('checks rooms and trims message text', () => {
    expect(assertChatRoom('player')).toBe('player')
    expect(() => assertChatRoom('nope')).toThrow('Invalid chat room.')
    expect(assertChatText('hello')).toBe('hello')
    expect(assertChatText('  hi  ')).toBe('hi')
    expect(() => assertChatText('   ')).toThrow('Type a message first.')
    expect(() => assertChatText('x'.repeat(141))).toThrow(
      'Chat messages are limited to 140 characters.',
    )
  })

  it('caps private messages at 8,000 characters', () => {
    expect(assertMessageText('hi')).toBe('hi')
    expect(() => assertMessageText('  ')).toThrow('Type a message first.')
    expect(() => assertMessageText('x'.repeat(8001))).toThrow(
      'Messages are limited to 8,000 characters.',
    )
  })
})

describe('tournaments', () => {
  it('checks new arenas, reasons, systems and ids', () => {
    expect(assertNewArena(NEW_ARENA)).toEqual(NEW_ARENA)
    expect(() => assertNewArena({})).toThrow('Invalid key')
    expect(assertDeclineReason('generic')).toBe('generic')
    expect(() => assertDeclineReason('nope')).toThrow('Invalid reason.')
    expect(assertTournamentSystem('arena')).toBe('arena')
    expect(assertTournamentSystem('swiss')).toBe('swiss')
    expect(() => assertTournamentSystem('x')).toThrow('Invalid tournament.')
    expect(assertTournamentId('abcd1234')).toBe('abcd1234')
    expect(() => assertTournamentId('abcd1234!')).toThrow('Invalid tournament.')
  })

  it('treats an empty or missing password as no password', () => {
    expect(assertTournamentPassword('')).toBeUndefined()
    expect(assertTournamentPassword(undefined)).toBeUndefined()
    expect(assertTournamentPassword('secret')).toBe('secret')
    expect(() => assertTournamentPassword('x'.repeat(101))).toThrow('Invalid password.')
  })
})

describe('exports, insights and Lichess ids', () => {
  it('keeps the caller’s bytes on an export', () => {
    const pgn = assertExport({ name: 'game 1.pgn', kind: 'pgn', data: '1. e4 *' })
    expect(pgn).toMatchObject({ name: 'game 1.pgn', kind: 'pgn', data: '1. e4 *' })
    const gifBytes = new Uint8Array([71, 73, 70, 56, 57, 97, 1])
    const gif = assertExport({ name: 'anim', kind: 'gif', data: gifBytes })
    expect(gif).toMatchObject({ name: 'anim', kind: 'gif' })
    expect(gif.data).toBe(gifBytes)
    const pngBytes = new Uint8Array([137, 80, 78, 71, 13])
    const png = assertExport({ name: 'img', kind: 'png', data: pngBytes })
    expect(png.data).toBe(pngBytes)
    expect(() => assertExport({ name: 'x', kind: 'bogus', data: '1. e4 *' })).toThrow(
      'Invalid export.',
    )
    expect(() => assertExport({ name: 'anim', kind: 'gif', data: new Uint8Array([1, 2]) })).toThrow(
      'Invalid export.',
    )
  })

  it('checks insights queries, perf types and Lichess ids', () => {
    const insights = { account: 'abc', speed: 'blitz', rated: true, days: 30 }
    expect(assertInsightsQuery(insights)).toEqual(insights)
    expect(() => assertInsightsQuery({})).toThrow('Invalid insights filter.')
    expect(assertPerfType('blitz')).toBe('blitz')
    expect(assertPerfType('crazyhouse')).toBe('crazyhouse')
    expect(() => assertPerfType('nope')).toThrow('Invalid rating category.')
    expect(assertLichessId('abcd1234')).toBe('abcd1234')
    expect(() => assertLichessId('Ab3')).toThrow('Invalid id.')
  })

  it('checks watch targets against the caller’s channels', () => {
    expect(assertWatchTarget({ channel: 'player' }, ['player', 'spectator'])).toEqual({
      channel: 'player',
    })
    expect(assertWatchTarget({ gameId: 'abcdefgh' }, ['player'])).toEqual({
      gameId: 'abcdefgh',
    })
    expect(() => assertWatchTarget({ channel: 'nope' }, ['player'])).toThrow(
      'Invalid game to watch.',
    )
    expect(() => assertWatchTarget({ gameId: 'abc' }, ['player'])).toThrow('Invalid game id.')
  })

  it('allows an empty account for “none”', () => {
    expect(assertOptionalAccount('')).toBe('')
    expect(assertOptionalAccount('abc')).toBe('abc')
    expect(() => assertOptionalAccount('!')).toThrow('Enter a valid Lichess username.')
  })
})

describe('game options, themes and notifications', () => {
  it('checks online options, themes and notifications', () => {
    const options = {
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
    expect(assertOnlineOptions(options)).toEqual(options)
    expect(() => assertOnlineOptions({})).toThrow('Invalid game options.')
    const theme = {
      id: 'midnight',
      name: 'Midnight',
      light: { bg: '#fff', text: '#000', primary: '#123456' },
      dark: { bg: '#000', text: '#fff', primary: '#abc', muted: '#111' },
    }
    expect(assertTheme(theme)).toEqual(theme)
    expect(() => assertTheme({})).toThrow('A theme file must be a JSON object.')
    const notification = { kind: 'test', title: 'Hi', body: 'There' }
    expect(assertNotification(notification)).toEqual(notification)
    expect(() => assertNotification({})).toThrow('Invalid notification.')
  })

  it('checks core and renderer settings', () => {
    expect(assertCoreSettings(CORE_SETTINGS)).toEqual(CORE_SETTINGS)
    expect(() => assertCoreSettings({})).toThrow('Invalid settings.')
    const settings = {
      ...CORE_SETTINGS,
      appearance: 'system',
      boardTheme: 'classic',
      lightTheme: 'light',
      darkTheme: 'dark',
      pieceSet: 'cburnett',
      pieceAnimation: 'normal',
      coordinates: 'inside',
      soundEnabled: true,
      soundVolume: 0.5,
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
      updateAutoCheck: true,
      updateAutoDownload: false,
      updateInstallOnQuit: true,
      engineLevels: ['club', 'gm'],
      reviewAuto: 'recent',
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
    expect(assertSettings(settings)).toEqual(settings)
    expect(() => assertSettings({})).toThrow('Invalid settings.')
  })
})

describe('puzzles and runs', () => {
  it('checks puzzle requests, solves and queries', () => {
    const request = { account: '', angle: 'mateIn2', difficulty: 'normal', color: 'white' }
    expect(assertPuzzleRequest(request)).toEqual(request)
    expect(() => assertPuzzleRequest({})).toThrow('Invalid puzzle request.')
    const solve = { account: 'abc', angle: 'mateIn2', id: 'abc123', win: true, rated: false }
    expect(assertPuzzleSolve(solve)).toEqual(solve)
    expect(() => assertPuzzleSolve({})).toThrow('Invalid puzzle result.')
    const query = { theme: 'mateIn2', minRating: 1000, maxRating: 2000, count: 10 }
    expect(assertLocalQuery(query)).toEqual(query)
    expect(() => assertLocalQuery({})).toThrow('Invalid puzzle query.')
    const ladder = { from: 1000, to: 2000, count: 10 }
    expect(assertLadderQuery(ladder)).toEqual(ladder)
    expect(() => assertLadderQuery({})).toThrow('Invalid puzzle query.')
  })

  it('checks run kinds and saved runs', () => {
    expect(assertRunKind('storm')).toBe('storm')
    expect(assertRunKind('streak')).toBe('streak')
    expect(() => assertRunKind('3min')).toThrow('Invalid kind of run.')
    expect(() => assertRunKind('nope')).toThrow('Invalid kind of run.')
    const run = { kind: 'storm', variant: 'standard', score: 12, detail: { a: 1, b: 'x', c: true } }
    expect(assertRunInput(run)).toEqual(run)
    expect(() => assertRunInput({})).toThrow('Invalid run.')
  })

  it('checks bounded numbers', () => {
    expect(assertDays(30)).toBe(30)
    expect(() => assertDays(0)).toThrow('Invalid number of days.')
    expect(() => assertDays(366)).toThrow('Invalid number of days.')
    expect(() => assertDays(1.5)).toThrow('Invalid number of days.')
    expect(assertActivityMax(200)).toBe(200)
    expect(() => assertActivityMax(0)).toThrow('Invalid number of puzzles.')
  })
})

describe('engine requests', () => {
  it('checks best-move options, defaulting to an empty request', () => {
    expect(assertBestMoveOptions({ fen: START, movetime: 500, chess960: false })).toEqual({
      fen: START,
      movetime: 500,
      chess960: false,
    })
    expect(assertBestMoveOptions({ movetime: 500 })).toEqual({ movetime: 500 })
    expect(assertBestMoveOptions(undefined)).toEqual({})
    expect(() => assertBestMoveOptions({ fen: 'bad' })).toThrow('Invalid position.')
    expect(() => assertBestMoveOptions({ movetime: -1 })).toThrow('Invalid think time.')
  })

  it('checks analysis requests, including repetition context', () => {
    expect(assertAnalysisRequest({ fen: START, lines: 3 })).toEqual({ fen: START, lines: 3 })
    const withContext = { fen: START, lines: 3, rootFen: START, moves: [], clientId: 1 }
    expect(assertAnalysisRequest(withContext)).toEqual(withContext)
    expect(() =>
      assertAnalysisRequest({ fen: START, lines: 3, rootFen: START, moves: ['e2e4'] }),
    ).toThrow('Invalid analysis position or move history.')
    expect(() => assertAnalysisRequest({ fen: START, lines: 0 })).toThrow(
      'Invalid number of engine lines.',
    )
    expect(() => assertAnalysisRequest({ fen: 'bad', lines: 3 })).toThrow('Invalid position.')
    expect(() => assertAnalysisRequest({})).toThrow('Invalid engine options.')
  })

  it('checks review requests and keys', () => {
    const review = { fen: START, moves: ['e2e4', 'e7e5'], gameId: 'abcdefgh', account: 'abc' }
    expect(assertReviewRequest(review)).toEqual(review)
    expect(() => assertReviewRequest({})).toThrow('Invalid review request.')
    expect(() =>
      assertReviewRequest({ fen: START, moves: ['bogus'], gameId: 'abcdefgh', account: 'abc' }),
    ).toThrow('Invalid move.')
    expect(assertReviewKey('0123456789abcdef')).toBe('0123456789abcdef')
    expect(() => assertReviewKey('xyz')).toThrow('Invalid review.')
  })
})

describe('voice, pages, studies and lookup', () => {
  it('checks voice attempts, updates, ids and limits', () => {
    const attempt = {
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
    expect(assertVoiceAttempt(attempt)).toEqual(attempt)
    expect(() => assertVoiceAttempt({})).toThrow('Invalid voice entry.')
    expect(assertVoiceUpdate({ outcome: 'correct', expected: 'e4' })).toEqual({
      outcome: 'correct',
      expected: 'e4',
    })
    expect(assertVoiceUpdate({})).toEqual({})
    expect(() => assertVoiceUpdate({ outcome: 'bogus' })).toThrow('Invalid voice entry.')
    expect(assertVoiceId(5)).toBe(5)
    expect(() => assertVoiceId(0)).toThrow('Invalid voice entry.')
    expect(() => assertVoiceId(1.5)).toThrow('Invalid voice entry.')
    expect(assertVoiceLimit(5000)).toBe(5000)
    expect(assertVoiceLimit(1)).toBe(1)
    expect(() => assertVoiceLimit(0)).toThrow('Invalid number of entries.')
  })

  it('checks game pages, study sync, broadcast search and lookup', () => {
    const page = { account: 'abc', result: 'win', rated: true, offset: 0, limit: 10 }
    expect(assertGamePageQuery(page)).toEqual(page)
    expect(() => assertGamePageQuery({})).toThrow('Invalid key')
    const sync = { account: 'abc', studyId: 'abcdefgh', baseline: '1. e4 *', pgn: '1. d4 *' }
    expect(assertStudySyncRequest(sync)).toEqual(sync)
    expect(() => assertStudySyncRequest({})).toThrow('Enter a valid Lichess username.')
    expect(assertBroadcastQuery('abc')).toBe('abc')
    expect(assertBroadcastQuery('  x  ')).toBe('x')
    expect(assertBroadcastQuery(undefined)).toBeUndefined()
    const lookup = {
      speeds: ['blitz'],
      ratings: [1600],
      player: 'abc',
      color: 'white',
      modes: ['rated'],
      since: '2020-01',
    }
    expect(assertLookupOptions(lookup)).toEqual(lookup)
    expect(assertLookupOptions(undefined)).toEqual({})
    expect(() => assertLookupOptions({ speeds: ['bogus'] })).toThrow('Invalid type')
  })
})

describe('shared patterns and levels', () => {
  it('re-exports the formats other layers match against', () => {
    expect(USERNAME.test('abc')).toBe(true)
    expect(USERNAME.test('a')).toBe(false)
    expect(GAME_ID.test('abcdefgh')).toBe(true)
    expect(GAME_ID.test('abc')).toBe(false)
    expect(PUZZLE_ID.test('abc123')).toBe(true)
    expect(UCI_MOVE.test('e2e4')).toBe(true)
    expect(UCI_MOVE.test('e2e9')).toBe(false)
    expect(PUZZLE_ANGLE.test('mateIn2')).toBe(true)
    expect(FEN.test(START)).toBe(true)
    expect(FEN.test('nonsense')).toBe(false)
  })

  it('re-exports the engine levels and online actions', () => {
    expect(ENGINE_LEVELS).toContain('beginner')
    expect(ENGINE_LEVELS).toContain('club')
    expect(ENGINE_LEVELS).toContain('max')
    expect(ONLINE_ACTIONS).toContain('resign')
    expect(ONLINE_ACTIONS).toContain('abort')
  })
})
