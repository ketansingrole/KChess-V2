import { beforeEach, expect, it, vi } from 'vitest'
import type { BrowserWindow, IpcMainInvokeEvent } from 'electron'
import { IPC_CHANNELS, type InvokeMethod } from '../../src/shared/ipc'
import { IPC_CONTRACTS, validateIpcArguments } from '../../src/main/ipcContracts'
import { INITIAL_FEN } from 'chessops/fen'

const mocks = vi.hoisted(() => ({ handle: vi.fn() }))
vi.mock('electron', () => ({ ipcMain: { handle: mocks.handle } }))
vi.mock('../../src/main/spectate', () => ({ TV_CHANNEL_KEYS: ['rapid', 'blitz'] }))

beforeEach(() => {
  mocks.handle.mockClear()
})

it('requires an explicit contract for every invocation and rejects excess arguments', () => {
  expect(Object.keys(IPC_CONTRACTS).sort()).toEqual(Object.keys(IPC_CHANNELS).sort())
  for (const method of Object.keys(IPC_CHANNELS) as InvokeMethod[]) {
    const contract = IPC_CONTRACTS[method]
    expect(() => validateIpcArguments(method, Array(contract.checks.length + 1))).toThrow(
      'argument count',
    )
    if (contract.min > 0) expect(() => validateIpcArguments(method, [])).toThrow('argument count')
  }
})
it.each([
  ['addAccount', ['../tokens']],
  ['playOnline', ['Game0001', 'garbage']],
  ['microphoneAccess', ['yes']],
  ['startAnalysis', [{ fen: 'invalid', lines: 1 }]],
  ['positionLookup', ['opening', INITIAL_FEN, { speeds: ['bad'] }]],
  ['reviewGet', [INITIAL_FEN, ['e2e5']]],
  ['recordPerformance', ['app.ready', Infinity]],
  ['recordPerformance', ['page.navigation:/players?name=Alice', 10]],
  ['reportRendererError', [{ route: '/players?name=Alice', message: 'Failure', info: 'render' }]],
  ['reportRendererError', [{ route: '/analysis', message: 'x'.repeat(1001), info: 'render' }]],
  ['exportToLichessStudy', ['Alice', '', 'Chapter', ' ']],
] as [InvokeMethod, unknown[]][])('rejects malformed %s input', (method, args) => {
  expect(() => validateIpcArguments(method, args)).toThrow()
})
it('allows optional arguments and preserves valid invocations', () => {
  expect(() =>
    validateIpcArguments('recordPerformance', ['page.navigation:/analysis', 10]),
  ).not.toThrow()
  expect(() =>
    validateIpcArguments('reportRendererError', [
      { route: '/analysis', message: 'Failure', info: 'render' },
    ]),
  ).not.toThrow()
  expect(() => validateIpcArguments('syncGames', [])).not.toThrow()
  expect(() => validateIpcArguments('bestMove', [[], 'club'])).not.toThrow()
  expect(() => validateIpcArguments('positionLookup', ['opening', INITIAL_FEN])).not.toThrow()
  expect(() => validateIpcArguments('reviewGet', [INITIAL_FEN, ['e2e4']])).not.toThrow()
})

it('authenticates the owned top-level frame before validation or handler effects', async () => {
  vi.resetModules()
  const { handle, setIpcOwner, assertIpcComplete } = await import('../../src/main/ipc')
  const mainFrame = { url: 'kchess://app/' }
  const contents = { mainFrame }
  setIpcOwner(
    () => ({ isDestroyed: () => false, webContents: contents }) as unknown as BrowserWindow,
  )
  const effect = vi.fn(async () => ({ byAccount: {}, versus: {} }))
  handle('gameLibraryOverview', effect)
  const listener = mocks.handle.mock.calls[0]![1] as (
    event: IpcMainInvokeEvent,
    ...args: unknown[]
  ) => unknown
  const event = { sender: contents, senderFrame: mainFrame } as unknown as IpcMainInvokeEvent
  for (const invalid of [
    { ...event, sender: {} },
    { ...event, senderFrame: { url: mainFrame.url } },
  ])
    expect(() => listener(invalid as IpcMainInvokeEvent)).toThrow('cannot access')
  expect(() => listener(event, 'extra')).toThrow('argument count')
  expect(effect).not.toHaveBeenCalled()
  await listener(event)
  expect(effect).toHaveBeenCalledTimes(1)
  expect(() => handle('gameLibraryOverview', effect)).toThrow('Duplicate')
  expect(() => assertIpcComplete()).toThrow('Missing desktop handlers')
  for (const method of Object.keys(IPC_CHANNELS) as InvokeMethod[])
    if (method !== 'gameLibraryOverview') handle(method, vi.fn())
  expect(() => assertIpcComplete()).not.toThrow()
})
