import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { testPlatform } from '../../../tests/fixtures/corePlatform'
import { INITIAL_FEN } from '../../src/domain/position'
import { pickMacAsset, pickStockfishAsset } from '../../src/services/stockfishAsset'
import { createKChessCore } from '../../src/services/service'

// The engine services run in the Rust core; these tests drive them through the core's API.
const dataDir = mkdtempSync(join(tmpdir(), 'kchess-engine-wrappers-'))
const core = createKChessCore(
  testPlatform({ dataDir, bundledEnginePath: join(dataDir, 'no-engine.js') }),
)
// Assistance waits until the core has confirmed there is no live game (as the desktop starts it).
await core.resumeOnline()

afterAll(async () => {
  await core.close()
  rmSync(dataDir, { recursive: true, force: true })
})

describe('engine status and trust', () => {
  it('reports no engine when neither the configured executable nor the bundled script exists', async () => {
    const status = await core.engineStatus()
    expect(status).toMatchObject({ ready: false, bundled: false })
  })

  it('saves only the executable paths the host has trusted', async () => {
    const picked = join(dataDir, 'picked-engine')
    core.trustEnginePath(picked)
    await expect(
      core.saveSettings({ ...(await core.settings()), enginePath: picked }),
    ).resolves.toMatchObject({ enginePath: picked })
    await expect(
      core.saveSettings({ ...(await core.settings()), enginePath: join(dataDir, 'other-engine') }),
    ).rejects.toThrow('Choose the Stockfish executable with the file picker.')
  })

  it('refuses a computer move when no engine can be found', async () => {
    await expect(core.bestMove(['e2e4'], 'beginner')).rejects.toThrow(
      'Stockfish could not be found. Choose an engine in Settings.',
    )
  })
})

describe('engine asset picker', () => {
  const assets = [
    { name: 'stockfish-linux-x86-64-universal.tar.gz', browser_download_url: 'l', size: 1 },
    { name: 'stockfish-macos-m1-apple-silicon.tar', browser_download_url: 'm', size: 1 },
  ]

  it('picks the official build for the platform and CPU', () => {
    expect(pickStockfishAsset(assets, 'linux', 'x64')?.name).toBe(
      'stockfish-linux-x86-64-universal.tar.gz',
    )
    expect(pickMacAsset(assets, 'arm64')?.name).toBe('stockfish-macos-m1-apple-silicon.tar')
    expect(pickStockfishAsset(assets, 'linux', 'ia32')).toBeUndefined()
  })
})

describe('analysis and review', () => {
  it('reports no analysis running and refuses an invalid request', async () => {
    await core.stopAnalysis()
    await expect(core.startAnalysis({ fen: 'startpos', lines: 1 })).rejects.toThrow()
  })

  it('reports review status and answers the stored state', async () => {
    expect(await core.reviewStatus()).toMatchObject({ waiting: 0 })
    expect(await core.reviewGet(INITIAL_FEN, [])).toBeNull()
    await core.stopEngine()
  })
})
