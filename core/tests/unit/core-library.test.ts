import { afterAll, beforeEach, describe, expect, it } from 'vitest'
import { useTestPlatform } from '../../../tests/fixtures/corePlatform'
import { closeDb, getDb } from '../../src/services/db'
import {
  clearRepertoireMisses,
  forgetTournamentsOf,
  importLibrary,
  joinedTournaments,
  library,
  readSession,
  recordRepertoireMiss,
  rememberTournament,
  removeArchivedGame,
  saveArchivedGame,
  saveSession,
  studyCommand,
} from '../../src/services/library'
import { assertSession, type ArchivedGame } from '../../src/domain/library'
import { STANDARD_SETUP } from '../../src/domain/variant'

useTestPlatform()
afterAll(closeDb)
beforeEach(() => getDb().exec('DELETE FROM documents; DELETE FROM archived_games;'))

const game = (id: string, moves = ['e2e4']): ArchivedGame => ({
  id,
  source: 'board',
  startedAt: 1,
  updatedAt: 1,
  white: 'White',
  black: 'Black',
  result: '*',
  reason: 'In progress',
  finished: false,
  setup: STANDARD_SETUP,
  moves,
  timeControl: '-',
})

describe('import from an earlier release', () => {
  it('takes over valid documents once, skips invalid ones and keeps what the library has', () => {
    expect(library().imported).toBe(false)
    const snapshot = importLibrary({
      'kchess:studies:v1': JSON.stringify({
        version: 1,
        items: [{ id: 'old', name: 'Old study', pgn: '1. e4 *', updatedAt: 1 }],
      }),
      'kchess:game-history:v1': JSON.stringify({ version: 1, games: [game('b'), game('a')] }),
      'kchess:computer:v1': JSON.stringify({ version: 1, moves: ['e2e5'], level: 'club' }),
      'kchess:analysis:v1': '{not json',
      'kchess:history-session:board': JSON.stringify({ id: 'board-1', startedAt: 5 }),
    })
    expect(snapshot.imported).toBe(true)
    expect(snapshot.studies.map((s) => s.chapters[0]!.id)).toEqual(['old'])
    // The most recent game stays most recent.
    expect(snapshot.games.map((g) => g.id)).toEqual(['b', 'a'])
    expect(snapshot.sessions.computer).toBeUndefined()
    expect(snapshot.sessions.analysis).toBeUndefined()
    expect(snapshot.sessions['archive:board']).toEqual({ id: 'board-1', startedAt: 5 })
    const again = importLibrary({
      'kchess:studies:v1': JSON.stringify({ version: 2, items: [] }),
    })
    expect(again.studies).toHaveLength(1)
  })
})

describe('studies', () => {
  it('keeps the library unchanged when a command fails', () => {
    const { id } = studyCommand({
      op: 'saveChapters',
      name: 'Repertoire',
      chapters: [{ name: 'One', pgn: '1. e4 *' }],
    })
    const before = library().studies
    expect(() =>
      studyCommand({
        op: 'saveChapters',
        name: 'Broken',
        chapters: [{ name: 'x', pgn: '1. e5 *' }],
      }),
    ).toThrow('valid chapters')
    expect(() =>
      studyCommand({ op: 'removeChapter', id: id!, chapterId: before[0]!.chapters[0]!.id }),
    ).toThrow('at least one chapter')
    expect(library().studies).toEqual(before)
  })

  it('refuses a 51st study', () => {
    for (let n = 0; n < 50; n++) studyCommand({ op: 'save', name: `Study ${n}`, pgn: '1. e4 *' })
    expect(() => studyCommand({ op: 'save', name: 'One more', pgn: '1. e4 *' })).toThrow('full')
    expect(library().studies).toHaveLength(50)
  })
})

describe('played games', () => {
  it('lists the most recently saved game first and replaces games in place', () => {
    saveArchivedGame(game('a'))
    saveArchivedGame(game('b'))
    saveArchivedGame({ ...game('a'), moves: ['e2e4', 'e7e5'] })
    expect(library().games.map((g) => [g.id, g.moves.length])).toEqual([
      ['a', 2],
      ['b', 1],
    ])
    removeArchivedGame('a')
    expect(library().games.map((g) => g.id)).toEqual(['b'])
  })

  it('refuses a game that would take the history past its size, by its old encoded size', () => {
    // Legal long games (knights back and forth) reach the size limit before the game limit.
    const shuffle = ['g1f3', 'g8f6', 'f3g1', 'f6g8']
    const moves = Array.from({ length: 1024 }, (_, n) => shuffle[n % 4]!)
    let saved = 0
    for (;;) {
      try {
        saveArchivedGame(game(String(saved), moves))
        saved++
      } catch (cause) {
        expect(String(cause)).toContain('full')
        break
      }
    }
    const { count, bytes } = getDb()
      .prepare('SELECT COUNT(*) AS count, SUM(length(body)) AS bytes FROM archived_games')
      .get() as { count: number; bytes: number }
    expect(count).toBe(saved)
    expect(saved).toBeLessThan(500)
    const document = JSON.stringify({ version: 1, games: [] }).length + bytes + count - 1
    expect(document).toBeLessThanOrEqual(2_000_000)
    expect(document + JSON.stringify(game('next', moves)).length + 1).toBeGreaterThan(2_000_000)
  })

  it('never drops older games for a new one', () => {
    for (let n = 0; n < 500; n++) saveArchivedGame(game(String(n)))
    expect(() => saveArchivedGame(game('overflow'))).toThrow('full')
    saveArchivedGame({ ...game('0'), result: '1-0', finished: true })
    expect(library().games).toHaveLength(500)
    expect(library().games[0]).toMatchObject({ id: '0', result: '1-0' })
  })
})

describe('sessions', () => {
  it('stores a session with its version and rejects an illegal one', () => {
    const local = assertSession('local', {
      setup: STANDARD_SETUP,
      moves: ['e2e4'],
      clock: null,
      times: null,
      result: null,
    })
    saveSession('local', local)
    expect(readSession('local')).toEqual(local)
    expect(() => assertSession('local', { ...local, moves: ['e2e5'] })).toThrow()
    expect(() => assertSession('computer', { moves: [], level: 'nope' })).toThrow()
  })
})

describe('tournaments and repertoire notes', () => {
  it('forgets ended tournaments and those of signed-out accounts', () => {
    const now = Date.now()
    rememberTournament({
      system: 'arena',
      id: 'ended',
      account: 'Alice',
      name: 'Old',
      until: now - 1,
    })
    rememberTournament({
      system: 'arena',
      id: 'live',
      account: 'Alice',
      name: 'Live',
      until: now + 60_000,
    })
    rememberTournament({
      system: 'swiss',
      id: 'other',
      account: 'Bob',
      name: 'Swiss',
      until: now + 60_000,
    })
    expect(joinedTournaments().map((t) => t.id)).toEqual(['live', 'other'])
    forgetTournamentsOf(['alice'])
    expect(joinedTournaments().map((t) => t.id)).toEqual(['other'])
  })

  it('counts missed repertoire moves per study and side', () => {
    const fen = 'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1'
    recordRepertoireMiss('study:white', fen)
    expect(recordRepertoireMiss('study:white', fen)).toEqual({ 'study:white': { [fen]: 2 } })
    expect(clearRepertoireMisses('study:white')).toEqual({})
  })
})

describe('analysis sessions', () => {
  it('validates a moved-along document and keeps the path it still contains', () => {
    const session = {
      pgn: '1. e4 e5 2. Nf3 *',
      path: '',
      orientation: 'white',
      study: '',
      chapter: '',
    }
    assertSession('analysis', session)
    expect(assertSession('analysis', { ...session, path: 'e2e4 e7e5' }).path).toBe('e2e4 e7e5')
    expect(assertSession('analysis', { ...session, path: 'e2e4 d7d5' }).path).toBe('e2e4')
    expect(() => assertSession('analysis', { ...session, pgn: '1. e5 *' })).toThrow()
  })
})
