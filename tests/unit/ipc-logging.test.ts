import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { BrowserWindow, IpcMainInvokeEvent } from 'electron'

const mocks = vi.hoisted(() => ({ handle: vi.fn() }))
vi.mock('electron', () => ({ ipcMain: { handle: mocks.handle } }))

function ownerWindow() {
  const mainFrame = { url: 'kchess://app/' }
  const contents = { mainFrame }
  return { mainFrame, contents }
}

describe('IPC failure logging', () => {
  beforeEach(() => {
    mocks.handle.mockClear()
    vi.resetModules()
  })

  it('logs handler failures with method and duration, without args, and rethrows', async () => {
    const { handle, setIpcOwner } = await import('../../src/main/ipc')
    const { mainFrame, contents } = ownerWindow()
    setIpcOwner(
      () => ({ isDestroyed: () => false, webContents: contents }) as unknown as BrowserWindow,
    )
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const failure = Object.assign(new Error('offline'), { status: 503, endpoint: 'GET /api/x' })
    handle('syncGames', async () => {
      throw failure
    })
    const listener = mocks.handle.mock.calls[0]![1] as (
      event: IpcMainInvokeEvent,
      ...args: unknown[]
    ) => Promise<unknown>
    const event = { sender: contents, senderFrame: mainFrame } as unknown as IpcMainInvokeEvent
    await expect(listener(event, 'Alice')).rejects.toThrow('offline')
    expect(warn).toHaveBeenCalledOnce()
    const line = String(warn.mock.calls[0]![0])
    expect(line).toContain('[ipc]')
    expect(line).toContain('syncGames')
    expect(line).toContain('durationMs=')
    expect(line).toContain('status=503')
    // Secrets and bodies never reach the log: method + error summary only.
    expect(line).not.toContain('Alice')
  })

  it('logs cancellations at debug level and successes silently', async () => {
    const { handle, setIpcOwner } = await import('../../src/main/ipc')
    const { mainFrame, contents } = ownerWindow()
    setIpcOwner(
      () => ({ isDestroyed: () => false, webContents: contents }) as unknown as BrowserWindow,
    )
    const debug = vi.spyOn(console, 'log').mockImplementation(() => {})
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    handle('stopAnalysis', async () => {
      throw new DOMException('aborted', 'AbortError')
    })
    const listener = mocks.handle.mock.calls[0]![1] as (
      event: IpcMainInvokeEvent,
    ) => Promise<unknown>
    const event = { sender: contents, senderFrame: mainFrame } as unknown as IpcMainInvokeEvent
    await expect(listener(event)).rejects.toThrow()
    expect(debug).toHaveBeenCalledOnce()
    expect(warn).not.toHaveBeenCalled()
  })
})
