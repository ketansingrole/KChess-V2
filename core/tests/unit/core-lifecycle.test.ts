import { afterEach, expect, it } from 'vitest'
import { createKChessCore, type KChessCore } from '../../src/services'
import { bindPlatform, platform } from '../../src/services/platform'
import { testPlatform } from '../../../tests/fixtures/corePlatform'

const first = testPlatform(),
  second = testPlatform()
let core: KChessCore | undefined
afterEach(async () => {
  await core?.close()
})

it('reopens a fresh profile without carrying accounts, settings or trusted executables over', async () => {
  core = createKChessCore(first)
  await core.addAccount('ProfileA')
  const data = await core.loadData()
  await core.saveSettings({ ...data.settings, cloudEval: true })
  core.trustEnginePath('/opt/profile-a/stockfish')
  const old = core
  await core.close()
  core = createKChessCore(second)
  const fresh = await core.loadData()
  expect(fresh.accounts).toEqual([])
  expect(fresh.settings.cloudEval).toBe(false)
  await expect(
    core.saveSettings({ ...fresh.settings, enginePath: '/opt/profile-a/stockfish' }),
  ).rejects.toThrow('file picker')
  await expect(old.addAccount('WrongProfile')).rejects.toThrow('closed')
  await old.close()
  expect((await core.loadData()).accounts).toEqual([])
  await core.close()
  core = createKChessCore(first)
  expect((await core.loadData()).accounts.map((a) => a.username)).toEqual(['ProfileA'])
})

it('binds late asynchronous work to the closed host, even after another profile opens', async () => {
  core = createKChessCore(testPlatform())
  const late = bindPlatform(() => platform().dataDir)
  await core.close()
  core = createKChessCore(testPlatform())
  expect(late).toThrow('closed')
})

it('direct core calls enforce argument tuples without relying on IPC', async () => {
  core = createKChessCore(testPlatform())
  const raw = core as unknown as {
    loadData(...args: unknown[]): Promise<unknown>
    recentGames(...args: unknown[]): Promise<unknown>
  }
  await expect(raw.loadData('extra')).rejects.toThrow('argument count')
  await expect(raw.recentGames('TestPlayer', 'yes')).rejects.toThrow()
})

it('isolates two direct cores even when methods are first read after both are created', async () => {
  const a = createKChessCore(testPlatform())
  const b = createKChessCore(testPlatform())
  try {
    await Promise.all([a.addAccount('ProfileA'), b.addAccount('ProfileB')])
    const original = await a.settings()
    await a.saveSettings({ ...original, cloudEval: true })
    a.trustEnginePath('/opt/profile-a/stockfish')
    expect((await b.settings()).cloudEval).toBe(false)
    await expect(
      b.saveSettings({ ...(await b.settings()), enginePath: '/opt/profile-a/stockfish' }),
    ).rejects.toThrow('file picker')
    await a.studyCommand({ op: 'save', name: 'OnlyA', pgn: '1. e4 *' })
    expect((await b.library()).studies).toEqual([])
    expect((await a.loadData()).accounts.map((a) => a.username)).toEqual(['ProfileA'])
    expect((await b.loadData()).accounts.map((a) => a.username)).toEqual(['ProfileB'])
    await a.close()
    await expect(a.settings()).rejects.toThrow('closed')
    await b.studyCommand({ op: 'save', name: 'StillB', pgn: '1. d4 *' })
    expect((await b.library()).studies.map((s) => s.name)).toEqual(['StillB'])
  } finally {
    await Promise.all([a.close(), b.close()])
  }
})

it('keeps desktop preferences out of core settings and preserves them on headless writes', async () => {
  core = createKChessCore(testPlatform())
  await core.saveDesktopSettings({
    ...(await core.desktopSettings()),
    boardTheme: 'blue',
    soundVolume: 0.2,
  })
  const settings = await core.settings()
  expect(settings).not.toHaveProperty('boardTheme')
  expect((await core.loadData()).settings).not.toHaveProperty('soundVolume')
  await core.saveSettings({ ...settings, cloudEval: true })
  expect(await core.desktopSettings()).toMatchObject({
    boardTheme: 'blue',
    soundVolume: 0.2,
    cloudEval: true,
  })
  await expect(core.saveSettings({ ...settings, reviewAuto: 'invalid' } as never)).rejects.toThrow()
})

it('creates an independent core from inside another core event callback', async () => {
  const a = createKChessCore(testPlatform())
  let b: KChessCore | undefined
  const off = a.on('settings:saved', () => {
    b = createKChessCore(testPlatform())
  })
  try {
    await a.saveSettings({ ...(await a.settings()), cloudEval: true })
    expect(b).toBeDefined()
    await b!.addAccount('NestedProfile')
    expect((await a.loadData()).accounts).toEqual([])
    expect((await b!.loadData()).accounts.map((account) => account.username)).toEqual([
      'NestedProfile',
    ])
  } finally {
    off()
    await Promise.all([a.close(), b?.close()])
  }
})
