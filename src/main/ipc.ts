import { ipcMain, type IpcMainInvokeEvent, type BrowserWindow } from 'electron'
import { IPC_CHANNELS, type InvokeMethod, type IpcInput, type IpcResult } from '../shared/ipc'

import { isAppUrl } from './appOrigin'
import { validateIpcArguments } from './ipcContracts'
import { errorSummary, isExpectedCancellation, logDebug, logWarn } from './logger'
import { recordTiming } from './performance'

const registered = new Set<InvokeMethod>()

export function assertIpcComplete(): void {
  const missing = (Object.keys(IPC_CHANNELS) as InvokeMethod[]).filter(
    (method) => !registered.has(method),
  )
  if (missing.length) throw new Error(`Missing desktop handlers: ${missing.join(', ')}`)
}

let owner: () => BrowserWindow | null = () => null
export function setIpcOwner(getWindow: () => BrowserWindow | null): void {
  owner = getWindow
}

/** Inputs remain unknown until each handler validates them; results follow DesktopApi. */
export function handle<K extends InvokeMethod>(
  method: K,
  listener: (
    event: IpcMainInvokeEvent,
    ...args: IpcInput<K>
  ) => Awaited<IpcResult<K>> | IpcResult<K>,
): void {
  if (registered.has(method)) throw new Error(`Duplicate desktop handler: ${method}`)
  ipcMain.handle(IPC_CHANNELS[method], (event, ...args: IpcInput<K>) => {
    const started = Date.now()
    const finish = (outcome: 'ok' | 'error', cause?: unknown): void => {
      const durationMs = Date.now() - started
      recordTiming(`ipc.${method}`, durationMs)
      if (outcome === 'error' && cause !== undefined) {
        // Never log args: they can carry FENs, PGNs, or user content.
        // Method + duration + error summary is enough to triage from logs alone.
        if (isExpectedCancellation(cause)) {
          logDebug(
            'ipc',
            `IPC ${method} cancelled:`,
            `durationMs=${durationMs}`,
            errorSummary(cause),
          )
        } else {
          logWarn('ipc', `IPC ${method} failed:`, `durationMs=${durationMs}`, errorSummary(cause))
        }
      }
    }
    const window = owner()
    if (
      !window ||
      window.isDestroyed() ||
      event.sender !== window.webContents ||
      event.senderFrame !== event.sender.mainFrame ||
      !isAppUrl(event.senderFrame.url)
    ) {
      const cause = new Error('This page cannot access KChess desktop services.')
      finish('error', cause)
      throw cause
    }
    try {
      validateIpcArguments(method, args)
    } catch (cause) {
      finish('error', cause)
      throw cause
    }
    let result: Awaited<IpcResult<K>> | IpcResult<K>
    try {
      result = listener(event, ...args)
    } catch (cause) {
      finish('error', cause)
      throw cause
    }
    return Promise.resolve(result).then(
      (value) => {
        finish('ok')
        return value
      },
      (cause) => {
        finish('error', cause)
        throw cause
      },
    )
  })
  registered.add(method)
}
