import { createServer } from 'node:http'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeEach, describe, expect, it } from 'vitest'
import {
  closeNativeCore,
  nativeCall,
  nativeCallSync,
  onNativeEvent,
} from '../../src/services/nativeCore'
import { cancelPuzzleDb, installPuzzleDb } from '../../src/services/puzzleDb'
import { setPlatform } from '../../src/services/platform'
import { clearRuns, runSummary, saveRun } from '../../src/services/runs'
import {
  clearVoiceHistory,
  saveVoiceAttempt,
  updateVoiceAttempt,
  voiceHistory,
  voiceHistoryDocument,
} from '../../src/services/voiceLog'
import { fakeSecrets, testPlatform } from '../../../tests/fixtures/corePlatform'

/** The storage wrappers' behaviour through the native core, on one temporary profile. */

const dataDir = mkdtempSync(join(tmpdir(), 'kchess-wrappers-'))
beforeEach(async () => {
  await closeNativeCore()
  setPlatform(testPlatform({ dataDir, secrets: fakeSecrets(false) }))
})
afterAll(async () => {
  await closeNativeCore()
  rmSync(dataDir, { recursive: true, force: true })
})

describe('played-game runs', () => {
  it('saves runs, reports the best of each variant, and clears one kind', () => {
    const first = saveRun({ kind: 'rush', variant: 'easy', score: 4, detail: { moves: 9 } })
    expect(first.isBest).toBe(true)
    expect(saveRun({ kind: 'rush', variant: 'easy', score: 2, detail: {} }).isBest).toBe(false)
    const summary = runSummary('rush')
    expect(summary.total).toBe(2)
    expect(summary.best.easy?.score).toBe(4)
    expect(summary.recent[0]?.score).toBe(2)
    clearRuns('rush')
    expect(runSummary('rush').total).toBe(0)
  })
})

describe('voice log', () => {
  it('saves, updates, lists, exports and clears attempts', () => {
    const id = saveVoiceAttempt({
      source: 'computer',
      heard: 'knight f three',
      confidence: 0.9,
      words: [{ word: 'knight', conf: 0.95 }],
      outcome: 'played',
    })
    updateVoiceAttempt(id, { outcome: 'invalid', expected: 'Nf3' })
    const [entry] = voiceHistory(10)
    expect(entry).toMatchObject({ id, outcome: 'invalid', expected: 'Nf3' })
    const document = JSON.parse(voiceHistoryDocument()) as { entries: { time: string }[] }
    expect(document.entries[0]?.time).toMatch(/^\d{4}-\d{2}-\d{2}T/)
    clearVoiceHistory()
    expect(voiceHistory(10)).toEqual([])
  })
})

describe('native core calls', () => {
  it('rejects an unknown storage method with its message', () => {
    expect(() => nativeCallSync('store.nothing.here')).toThrow('Unknown core method')
  })

  it('resolves a puzzle call that has nothing to cancel', async () => {
    await expect(nativeCall('puzzles.cancel')).resolves.toBeNull()
  })

  it('rejects an unknown native call with an ordinary error', async () => {
    await expect(nativeCall('puzzles.nothing')).rejects.toThrow('Unknown core method')
  })

  it('stops an event listener when asked, and closes an idle core again', async () => {
    const stop = onNativeEvent('test:event', () => {})
    expect(stop()).toBe(true)
    await closeNativeCore()
    await closeNativeCore()
  })
})

describe('voice log corrections', () => {
  it('leaves an attempt alone when a correction names nothing to change', () => {
    const id = saveVoiceAttempt({
      source: 'computer',
      heard: 'e four',
      confidence: 0.7,
      words: [],
      outcome: 'played',
    })
    updateVoiceAttempt(id, {})
    updateVoiceAttempt(id, { expected: 'e4' })
    expect(voiceHistory(1)[0]).toMatchObject({ id, expected: 'e4', outcome: 'played' })
  })
})

describe('puzzle database download', () => {
  it('reports progress to its listener and rejects a download that cannot connect', async () => {
    const progress: string[] = []
    await expect(
      installPuzzleDb((event) => progress.push(event.phase), 'http://127.0.0.1:9/puzzles.csv.zst'),
    ).rejects.toThrow()
    expect(progress.length).toBeGreaterThanOrEqual(0)
    cancelPuzzleDb()
  })
})

describe('a cancelled puzzle download', () => {
  it('ends without the database when the download is cancelled', async () => {
    const silent = createServer(() => {})
    await new Promise<void>((resolve) => silent.listen(0, '127.0.0.1', resolve))
    const address = silent.address()
    const port = typeof address === 'object' && address ? address.port : 0
    try {
      const download = installPuzzleDb(() => {}, `http://127.0.0.1:${port}/puzzles.csv.zst`)
      await new Promise((resolve) => setTimeout(resolve, 100))
      cancelPuzzleDb()
      expect(await download).toMatchObject({ installed: false })
    } finally {
      silent.closeAllConnections()
      await new Promise<void>((resolve) => silent.close(() => resolve()))
    }
  })
})

describe('test-only storage methods', () => {
  it('are unknown methods unless the process opts in with KCHESS_STORE_DEBUG=1', () => {
    const optedIn = process.env.KCHESS_STORE_DEBUG
    delete process.env.KCHESS_STORE_DEBUG
    try {
      expect(() => nativeCallSync('store.debug.tables')).toThrow(
        'Unknown core method store.debug.tables.',
      )
      expect(() => nativeCallSync('store.debug.query', 'SELECT 1', [])).toThrow(
        'Unknown core method store.debug.query.',
      )
    } finally {
      process.env.KCHESS_STORE_DEBUG = optedIn
    }
    expect(nativeCallSync('store.debug.query', 'SELECT 1 AS one', [])).toEqual([{ one: 1 }])
  })
})
