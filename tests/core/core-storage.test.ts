import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { createKChessCore, type KChessCore } from '@kchess/native'
import { testPlatform } from '../fixtures/corePlatform'

/** The storage methods of the core's public API, each through its validation and its store. */

const dataDir = mkdtempSync(join(tmpdir(), 'kchess-storage-api-'))
const core: KChessCore = createKChessCore(testPlatform({ dataDir }))
afterAll(async () => {
  await core.close()
  rmSync(dataDir, { recursive: true, force: true })
})

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'

describe('settings and accounts', () => {
  it('saves valid settings and refuses invalid ones', async () => {
    const settings = await core.desktopSettings()
    await core.saveDesktopSettings({ ...settings, zenMode: !settings.zenMode })
    expect((await core.desktopSettings()).zenMode).toBe(!settings.zenMode)
    await expect(
      core.saveDesktopSettings({ ...settings, zenMode: 'yes' } as never),
    ).rejects.toThrow()
    await expect(core.saveSettings({ promotion: 7 } as never)).rejects.toThrow()
  })

  it('refuses an engine executable the user did not pick', async () => {
    const settings = await core.desktopSettings()
    await expect(
      core.saveDesktopSettings({ ...settings, enginePath: '/tmp/not-picked' }),
    ).rejects.toThrow('Choose the Stockfish executable with the file picker.')
  })

  it('signs out and removes accounts that are not here', async () => {
    await core.logout('Nobody')
    await core.removeAccount('Nobody')
    expect((await core.loadData()).accounts).toEqual([])
  })

  it('follows, removes and clears accounts', async () => {
    await core.addFriends(['Bob'])
    await expect(core.addAccount('not a name!')).rejects.toThrow()
    await core.removeAccount('Bob')
    await core.clearAccountData('nobody')
    await core.logoutAll()
    expect((await core.loadData()).accounts).toEqual([])
  })

  it('reads game lists, overviews and PGNs, refusing malformed queries', async () => {
    await expect(core.gamePage({ offset: -1, limit: 10 } as never)).rejects.toThrow()
    expect(await core.gamePage({ offset: 0, limit: 10 })).toEqual({ games: [], total: 0 })
    expect((await core.gameLibraryOverview()).byAccount).toEqual({})
    expect(await core.gameRatingHistory('Nobody')).toEqual([])
    expect(await core.gamePgn('Nobody', 'Game0001')).toBeNull()
    expect(await core.insights({ account: 'Nobody' })).toMatchObject({})
    await expect(core.insights({ account: 'bad name!' } as never)).rejects.toThrow()
  })
})

describe('usage and runs', () => {
  it('reports and resets usage', async () => {
    const report = await core.usage()
    expect(report.accounts).toEqual({})
    await core.resetUsage()
  })

  it('saves, summarises and clears runs, refusing an unknown kind', async () => {
    const saved = await core.saveRun({ kind: 'storm', variant: '3min', score: 9, detail: {} })
    expect(saved.isBest).toBe(true)
    expect((await core.runSummary('storm')).total).toBe(1)
    await expect(core.runSummary('chess' as never)).rejects.toThrow()
    await core.clearRuns('storm')
    await core.clearRuns()
    expect((await core.runSummary('storm')).total).toBe(0)
  })
})

describe('voice log', () => {
  it('records, corrects, lists and clears attempts', async () => {
    const id = await core.saveVoiceAttempt({
      source: 'computer',
      heard: 'pawn e four',
      confidence: 0.8,
      words: [],
      outcome: 'played',
    })
    await core.updateVoiceAttempt(id, { outcome: 'invalid' })
    expect((await core.voiceHistory(5))[0]).toMatchObject({ id, outcome: 'invalid' })
    await core.clearVoiceHistory()
    expect(await core.voiceHistory(5)).toEqual([])
  })
})

describe('library', () => {
  it('reads the library, takes earlier documents over once, and saves studies', async () => {
    const before = await core.library()
    expect(before.studies).toEqual([])
    const imported = await core.importLibrary({})
    expect(imported.imported).toBe(true)
    const study = await core.studyCommand({ op: 'save', name: 'Openings', pgn: '1. e4 *' })
    expect(study.studies.map((item) => item.name)).toEqual(['Openings'])
    await expect(core.studyCommand({ op: 'save', name: '' } as never)).rejects.toThrow()
  })

  it('tracks mistake drills, joined tournaments and repertoire misses', async () => {
    await expect(core.addMistakes('no-such-review')).rejects.toThrow('Invalid review')
    expect(await core.answerMistake('no-such-drill', true)).toEqual([])
    expect(await core.joinedTournaments()).toEqual([])
    const misses = await core.recordRepertoireMiss('study-1', START)
    expect(misses['study-1']?.[START]).toBe(1)
    expect(await core.clearRepertoireMisses('study-1')).toEqual({})
  })

  it('reads the review summaries of games it does not have', async () => {
    expect(await core.reviewSummaries(['Game0001'])).toEqual({})
  })
})

describe('played games archive and followed players', () => {
  it('saves and removes an archived game', async () => {
    const game = {
      id: 'Board0001',
      source: 'board',
      startedAt: 1,
      updatedAt: 1,
      white: 'White',
      black: 'Black',
      result: '*',
      reason: 'In progress',
      finished: false,
      setup: { variant: 'standard', fen: START },
      moves: ['e2e4'],
      timeControl: '-',
    } as const
    await core.saveArchivedGame(game as never)
    expect((await core.library()).games.map((item) => item.id)).toEqual(['Board0001'])
    await core.removeArchivedGame('Board0001')
    expect((await core.library()).games).toEqual([])
    await expect(core.saveArchivedGame({ id: 'bad' } as never)).rejects.toThrow()
  })

  it('reads the followed players and cached profiles, and nothing for a stranger', async () => {
    expect((await core.following()).users ?? []).toEqual([])
    expect(await core.cachedProfile('Nobody')).toMatchObject({ profile: null })
  })
})

describe('refused storage input', () => {
  it('refuses malformed voice, library and repertoire input before storing it', async () => {
    await expect(core.saveVoiceAttempt({ source: 'nope' } as never)).rejects.toThrow()
    await expect(core.updateVoiceAttempt('x' as never, {})).rejects.toThrow()
    await expect(core.importLibrary({ 'kchess:studies:v1': 42 } as never)).rejects.toThrow()
    await expect(core.recordRepertoireMiss('study-1', 'not a fen')).rejects.toThrow()
    await expect(core.reviewSummaries(['bad id!'] as never)).rejects.toThrow()
  })

  it('reports the engine and the storm dashboard from the store', async () => {
    expect(await core.engineStatus()).toBeTruthy()
  })
})

describe('local puzzle queries', () => {
  it('refuses a malformed puzzle query before reading the database', async () => {
    await expect(core.localPuzzles({ rating: 'high' } as never)).rejects.toThrow()
    await expect(core.localLadder({ rating: 'high' } as never)).rejects.toThrow()
  })
})

describe('puzzle database status', () => {
  it('reports that no puzzle database is installed, and deletes nothing', async () => {
    expect(await core.puzzleDbStatus()).toMatchObject({ installed: false })
    await core.puzzleDbCancel()
    expect(await core.puzzleDbDelete()).toMatchObject({ installed: false })
  })
})
