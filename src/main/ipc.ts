import { ipcMain, type IpcMainInvokeEvent } from 'electron'
import { IPC_CHANNELS, type InvokeMethod, type IpcInput, type IpcResult } from '../shared/ipc'

/** Inputs remain unknown until each handler validates them; results follow DesktopApi. */
export function handle<K extends InvokeMethod>(
  method: K,
  listener: (
    event: IpcMainInvokeEvent,
    ...args: IpcInput<K>
  ) => Awaited<IpcResult<K>> | IpcResult<K>,
): void {
  ipcMain.handle(IPC_CHANNELS[method], listener)
}
