import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { useTestPlatform } from '../../../tests/fixtures/corePlatform'
import { INITIAL_FEN } from '../../src/domain/position'
import { closeNativeCore } from '../../src/services/nativeCore'

// The engine services run in the Rust core. These tests cover the TypeScript wrappers' own
// behaviour: status and trust, the budget, the asset picker, install and delete locations,
// cancellation through a signal, and the analysis and review forwarding.
const dataDir = mkdtempSync(join(tmpdir(), 'kchess-engine-wrappers-'))
useTestPlatform({ dataDir, bundledEnginePath: join(dataDir, 'no-engine.js') })
process.env.KCHESS_STORE_DEBUG = '1'

const engine = await import('../../src/services/engine')
const managed = await import('../../src/services/managedEngine')
const asset = await import('../../src/services/stockfishAsset')
const analysis = await import('../../src/services/analysis')
const review = await import('../../src/services/review')

afterAll(async () => {
  await closeNativeCore()
  rmSync(dataDir, { recursive: true, force: true })
})

describe('engine status and trust', () => {
  it('reports no engine when neither the configured executable nor the bundled script exists', async () => {
    const status = await engine.engineStatus(join(dataDir, 'missing-engine'))
    expect(status).toMatchObject({ ready: false, bundled: false })
  })

  it('trusts only the paths the host has picked', () => {
    const picked = join(dataDir, 'picked-engine')
    expect(engine.isTrustedEnginePath(picked)).toBe(false)
    engine.trustEnginePath(picked)
    expect(engine.isTrustedEnginePath(picked)).toBe(true)
    expect(engine.isTrustedEnginePath(join(dataDir, 'other-engine'))).toBe(false)
  })

  it('offers an engine budget and no computer move in progress', () => {
    expect(engine.engineThreads()).toBeGreaterThanOrEqual(1)
    expect(engine.computerPlaying(60_000)).toBe(false)
    engine.setEngineBusy(true)
    engine.setEngineBusy(false)
    engine.stopEngine()
    engine.resetEngine()
  })

  it('refuses a computer move when no engine can be found', async () => {
    await expect(
      engine.bestMove(['e2e4'], 'beginner', join(dataDir, 'missing-engine')),
    ).rejects.toThrow('Stockfish could not be found. Choose an engine in Settings.')
  })
})

describe('managed engine', () => {
  it('reports a missing engine and deletes an absent directory harmlessly', async () => {
    const dir = join(dataDir, 'managed-none')
    expect(await managed.managedEngine({ dir })).toMatchObject({ installed: false })
    await expect(managed.deleteManagedEngine({ dir })).resolves.toBeUndefined()
  })

  it('cancels an install whose signal aborts', async () => {
    const controller = new AbortController()
    const install = managed.installManagedEngine(
      { dir: join(dataDir, 'managed-cancel'), releaseUrl: 'http://127.0.0.1:9/release' },
      controller.signal,
    )
    controller.abort()
    await expect(install).rejects.toThrow()
  })
})

describe('engine asset picker', () => {
  const assets = [
    { name: 'stockfish-linux-x86-64-universal.tar.gz', browser_download_url: 'l', size: 1 },
    { name: 'stockfish-macos-m1-apple-silicon.tar', browser_download_url: 'm', size: 1 },
  ]

  it('picks the official build for the platform and CPU', () => {
    expect(asset.pickStockfishAsset(assets, 'linux', 'x64')?.name).toBe(
      'stockfish-linux-x86-64-universal.tar.gz',
    )
    expect(asset.pickMacAsset(assets, 'arm64')?.name).toBe('stockfish-macos-m1-apple-silicon.tar')
    expect(asset.pickStockfishAsset(assets, 'linux', 'ia32')).toBeUndefined()
  })
})

describe('analysis and review wrappers', () => {
  it('reports no analysis running and refuses an invalid request', async () => {
    expect(analysis.analysisRunning()).toBe(false)
    analysis.stopAnalysis(true)
    await expect(
      analysis.startAnalysis({ fen: 'startpos', lines: 1 }, '', () => {}),
    ).rejects.toThrow()
  })

  it('forwards review status and answers the stored state', async () => {
    const statuses: unknown[] = []
    review.setupReviews({
      update: () => {},
      status: (status) => statuses.push(status),
    })
    expect(review.reviewStatus()).toMatchObject({ waiting: 0 })
    expect(review.getReview(INITIAL_FEN, [])).toBeNull()
    review.cancelReview('no-such-review')
    review.discardAccountReviews([])
    review.reviewsChanged()
    review.restartReviewEngine()
    await review.stopReviews()
  })
})
