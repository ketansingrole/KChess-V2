import { ipcMain, type IpcMainInvokeEvent, type BrowserWindow } from 'electron'
import { IPC_CHANNELS, type InvokeMethod, type IpcInput, type IpcResult } from '../shared/ipc'

import { isAppUrl } from './appOrigin'
import { validateIpcArguments } from './ipcContracts'

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
    const window = owner()
    if (
      !window ||
      window.isDestroyed() ||
      event.sender !== window.webContents ||
      event.senderFrame !== event.sender.mainFrame ||
      !isAppUrl(event.senderFrame.url)
    )
      throw new Error('This page cannot access KChess desktop services.')
    validateIpcArguments(method, args)
    return listener(event, ...args)
  })
  registered.add(method)
}
