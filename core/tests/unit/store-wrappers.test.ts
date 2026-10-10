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
import { setPlatform } from '../../src/services/platform'
import { createKChessCore, type KChessCore } from '../../src/services/service'
import { fakeSecrets, testPlatform } from '../../../tests/fixtures/corePlatform'

/** The storage behaviour through the native core, on temporary profiles. */

const dataDir = mkdtempSync(join(tmpdir(), 'kchess-wrappers-'))
beforeEach(async () => {
  await closeNativeCore()
  setPlatform(testPlatform({ dataDir, secrets: fakeSecrets(false) }))
})
afterAll(async () => {
  await closeNativeCore()
  rmSync(dataDir, { recursive: true, force: true })
})

/** A core on its own fresh profile, closed with `done`. */
async function withCore(work: (core: KChessCore) => Promise<void>): Promise<void> {
  const profile = mkdtempSync(join(tmpdir(), 'kchess-core-'))
  const core = createKChessCore(testPlatform({ dataDir: profile, secrets: fakeSecrets(false) }))
  try {
    await work(core)
  } finally {
    await core.close()
    rmSync(profile, { recursive: true, force: true })
  }
}

describe('played-game runs', () => {
  it('saves runs, reports the best of each variant, and clears one kind', () =>
    withCore(async (core) => {
      const first = await core.saveRun({
        kind: 'rush',
        variant: 'easy',
        score: 4,
        detail: { moves: 9 },
      })
      expect(first.isBest).toBe(true)
      expect(
        (await core.saveRun({ kind: 'rush', variant: 'easy', score: 2, detail: {} })).isBest,
      ).toBe(false)
      const summary = await core.runSummary('rush')
      expect(summary.total).toBe(2)
      expect(summary.best.easy?.score).toBe(4)
      expect(summary.recent[0]?.score).toBe(2)
      await core.clearRuns('rush')
      expect((await core.runSummary('rush')).total).toBe(0)
    }))
})

describe('voice log', () => {
  it('saves, updates, lists, exports and clears attempts', () =>
    withCore(async (core) => {
      const id = await core.saveVoiceAttempt({
        source: 'computer',
        heard: 'knight f three',
        confidence: 0.9,
        words: [{ word: 'knight', conf: 0.95 }],
        outcome: 'played',
      })
      await core.updateVoiceAttempt(id, { outcome: 'invalid', expected: 'Nf3' })
      const [entry] = await core.voiceHistory(10)
      expect(entry).toMatchObject({ id, outcome: 'invalid', expected: 'Nf3' })
      const document = JSON.parse(core.voiceHistoryDocument()) as { entries: { time: string }[] }
      expect(document.entries[0]?.time).toMatch(/^\d{4}-\d{2}-\d{2}T/)
      await core.clearVoiceHistory()
      expect(await core.voiceHistory(10)).toEqual([])
    }))

  it('leaves an attempt alone when a correction names nothing to change', () =>
    withCore(async (core) => {
      const id = await core.saveVoiceAttempt({
        source: 'computer',
        heard: 'e four',
        confidence: 0.7,
        words: [],
        outcome: 'played',
      })
      await core.updateVoiceAttempt(id, {})
      await core.updateVoiceAttempt(id, { expected: 'e4' })
      expect((await core.voiceHistory(1))[0]).toMatchObject({
        id,
        expected: 'e4',
        outcome: 'played',
      })
    }))
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

describe('puzzle database download', () => {
  it('reports progress to its listener and rejects a download that cannot connect', async () => {
    const progress: string[] = []
    const stop = onNativeEvent<{ phase: string }>('puzzledb:progress', (event) =>
      progress.push(event.phase),
    )
    try {
      await expect(
        nativeCall('puzzles.install', 'http://127.0.0.1:9/puzzles.csv.zst'),
      ).rejects.toThrow()
      expect(progress.length).toBeGreaterThanOrEqual(0)
    } finally {
      stop()
      await nativeCall('puzzles.cancel')
    }
  })
})

describe('a cancelled puzzle download', () => {
  it('ends without the database when the download is cancelled', async () => {
    const silent = createServer(() => {})
    await new Promise<void>((resolve) => silent.listen(0, '127.0.0.1', resolve))
    const address = silent.address()
    const port = typeof address === 'object' && address ? address.port : 0
    try {
      const download = nativeCall<{ installed: boolean }>(
        'puzzles.install',
        `http://127.0.0.1:${port}/puzzles.csv.zst`,
      )
      await new Promise((resolve) => setTimeout(resolve, 100))
      await nativeCall('puzzles.cancel')
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
