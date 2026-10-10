import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { testPlatform } from '../../../tests/fixtures/corePlatform'
import { createKChessCore, type KChessCore } from '../../src/services/service'
import { assertSession, type ArchivedGame } from '../../src/domain/library'
import { STANDARD_SETUP } from '../../src/domain/variant'

// Each test opens its own profile, so the library starts empty.
let core: KChessCore
let dataDir: string
beforeEach(() => {
  dataDir = mkdtempSync(join(tmpdir(), 'kchess-library-'))
  core = createKChessCore(testPlatform({ dataDir }))
})
afterEach(async () => {
  await core.close()
  rmSync(dataDir, { recursive: true, force: true })
})

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
  it('takes over valid documents once, skips invalid ones and keeps what the library has', async () => {
    expect((await core.library()).imported).toBe(false)
    const snapshot = await core.importLibrary({
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
    const again = await core.importLibrary({
      'kchess:studies:v1': JSON.stringify({ version: 2, items: [] }),
    })
    expect(again.studies).toHaveLength(1)
  })
})

describe('studies', () => {
  it('keeps the library unchanged when a command fails', async () => {
    const { id } = await core.studyCommand({
      op: 'saveChapters',
      name: 'Repertoire',
      chapters: [{ name: 'One', pgn: '1. e4 *' }],
    })
    const before = (await core.library()).studies
    await expect(
      core.studyCommand({
        op: 'saveChapters',
        name: 'Broken',
        chapters: [{ name: 'x', pgn: '1. e5 *' }],
      }),
    ).rejects.toThrow('valid chapters')
    await expect(
      core.studyCommand({ op: 'removeChapter', id: id!, chapterId: before[0]!.chapters[0]!.id }),
    ).rejects.toThrow('at least one chapter')
    expect((await core.library()).studies).toEqual(before)
  })

  it('refuses a 51st study', async () => {
    for (let n = 0; n < 50; n++)
      await core.studyCommand({ op: 'save', name: `Study ${n}`, pgn: '1. e4 *' })
    await expect(
      core.studyCommand({ op: 'save', name: 'One more', pgn: '1. e4 *' }),
    ).rejects.toThrow('full')
    expect((await core.library()).studies).toHaveLength(50)
  })
})

describe('played games', () => {
  it('lists the most recently saved game first and replaces games in place', async () => {
    await core.saveArchivedGame(game('a'))
    await core.saveArchivedGame(game('b'))
    await core.saveArchivedGame({ ...game('a'), moves: ['e2e4', 'e7e5'] })
    expect((await core.library()).games.map((g) => [g.id, g.moves.length])).toEqual([
      ['a', 2],
      ['b', 1],
    ])
    await core.removeArchivedGame('a')
    expect((await core.library()).games.map((g) => g.id)).toEqual(['b'])
  })

  it('refuses a game once the history is full, and keeps what it saved', async () => {
    // Legal long games (knights back and forth) reach the size limit before the game limit.
    const shuffle = ['g1f3', 'g8f6', 'f3g1', 'f6g8']
    const moves = Array.from({ length: 1024 }, (_, n) => shuffle[n % 4]!)
    let saved = 0
    for (;;) {
      try {
        await core.saveArchivedGame(game(String(saved), moves))
        saved++
      } catch (cause) {
        expect(String(cause)).toContain('full')
        break
      }
    }
    expect(saved).toBeLessThan(500)
    expect((await core.library()).games).toHaveLength(saved)
  })

  it('never drops older games for a new one', async () => {
    for (let n = 0; n < 500; n++) await core.saveArchivedGame(game(String(n)))
    await expect(core.saveArchivedGame(game('overflow'))).rejects.toThrow('full')
    await core.saveArchivedGame({ ...game('0'), result: '1-0', finished: true })
    const games = (await core.library()).games
    expect(games).toHaveLength(500)
    expect(games[0]).toMatchObject({ id: '0', result: '1-0' })
  })
})

describe('sessions', () => {
  it('stores a session with its version and rejects an illegal one', async () => {
    const local = assertSession('local', {
      setup: STANDARD_SETUP,
      moves: ['e2e4'],
      clock: null,
      times: null,
      result: null,
    })
    await core.saveSession('local', local)
    expect((await core.library()).sessions.local).toEqual(local)
    await expect(core.saveSession('local', { ...local, moves: ['e2e5'] })).rejects.toThrow()
    // A malformed session is refused by the core, whatever its type says.
    await expect(
      core.saveSession('computer', { moves: [], level: 'nope' } as never),
    ).rejects.toThrow()
  })
})

describe('repertoire notes', () => {
  it('counts missed repertoire moves per study and side', async () => {
    const fen = 'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1'
    await core.recordRepertoireMiss('study:white', fen)
    expect(await core.recordRepertoireMiss('study:white', fen)).toEqual({
      'study:white': { [fen]: 2 },
    })
    expect(await core.clearRepertoireMisses('study:white')).toEqual({})
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
