import { afterAll, describe, expect, it } from 'vitest'
import { CORE_METHODS, createKChessCore, type KChessCore } from '../../src/services'
import type { CoreSettings } from '../../src/contracts/types'
import { testPlatform } from '../../../tests/fixtures/corePlatform'

const platform = testPlatform()
const core: KChessCore = createKChessCore(platform)

afterAll(() => core.close())

describe('headless core', () => {
  it('implements every CoreApi method', () => {
    for (const method of CORE_METHODS) expect(typeof core[method], method).toBe('function')
  })

  it('rejects a duplicate host for the same profile', () => {
    expect(() => createKChessCore(testPlatform({ dataDir: platform.dataDir }))).toThrow(
      'already running',
    )
  })

  it('stores data in the platform directory without a frontend', async () => {
    const data = await core.loadData()
    expect(data.accounts).toEqual([])
    const saved = await core.saveRun({ kind: 'storm', variant: '3min', score: 12, detail: {} })
    expect(saved).toBeTruthy()
    expect((await core.runSummary('storm')).best['3min']).toMatchObject({ score: 12 })
  })

  it('validates input itself', async () => {
    await expect(core.addAccount('not a username!')).rejects.toThrow()
    await expect(core.runSummary('chess' as never)).rejects.toThrow()
  })

  it('reports saved settings to subscribers until they unsubscribe', async () => {
    const seen: CoreSettings[] = []
    const stop = core.on('settings:saved', (settings) => seen.push(settings))
    const { settings } = await core.loadData()
    await core.saveSettings({ ...settings, cloudEval: !settings.cloudEval })
    stop()
    await core.saveSettings(settings)
    expect(seen.map((entry) => entry.cloudEval)).toEqual([!settings.cloudEval])
  })

  it('accepts only engine executables the user picked', async () => {
    const { settings } = await core.loadData()
    const picked = '/opt/engines/stockfish'
    await expect(core.saveSettings({ ...settings, enginePath: picked })).rejects.toThrow(
      'file picker',
    )
    core.trustEnginePath(picked)
    expect((await core.saveSettings({ ...settings, enginePath: picked })).enginePath).toBe(picked)
    await core.saveSettings(settings)
  })
})
