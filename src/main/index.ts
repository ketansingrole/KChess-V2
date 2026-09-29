import { app, BrowserWindow, dialog, ipcMain, shell } from 'electron'
import { join } from 'node:path'
import { bestMove, engineStatus, installStockfish } from './engine'
import { OnlineSession, connectLichess, profile, ratingHistory, syncGames } from './lichess'
import { addAccount, loadData, removeAccount, saveSettings } from './store'
import type { OnlineEvent, Settings } from '../shared/types'

let window: BrowserWindow | null = null
app.setName('KChess')
const online = new OnlineSession(
  (event: OnlineEvent) => window?.webContents.send('online:event', event),
  (message: string) => window?.webContents.send('online:error', message)
)

function createWindow(): void {
  window = new BrowserWindow({
    title: 'KChess', width: 1320, height: 880, minWidth: 880, minHeight: 620,
    backgroundColor: '#111827',
    webPreferences: { preload: join(__dirname, '../preload/index.cjs'), contextIsolation: true, nodeIntegration: false, sandbox: true }
  })
  window.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith('https://lichess.org/')) void shell.openExternal(url)
    return { action: 'deny' }
  })
  if (process.env.ELECTRON_RENDERER_URL) void window.loadURL(process.env.ELECTRON_RENDERER_URL)
  else void window.loadFile(join(__dirname, '../renderer/index.html'))
}

app.whenReady().then(() => {
  ipcMain.handle('data:load', () => loadData())
  ipcMain.handle('settings:save', (_event, settings: Settings) => saveSettings(settings))
  ipcMain.handle('account:add', (_event, username: string) => addAccount(username))
  ipcMain.handle('account:remove', (_event, username: string) => removeAccount(username))
  ipcMain.handle('games:sync', (_event, username?: string) => syncGames(username))
  ipcMain.handle('lichess:profile', (_event, username: string) => profile(username))
  ipcMain.handle('lichess:rating', (_event, username: string) => ratingHistory(username))
  ipcMain.handle('lichess:connect', () => connectLichess())
  ipcMain.handle('engine:status', (_event, path?: string) => engineStatus(path))
  ipcMain.handle('engine:choose', async () => {
    const result = await dialog.showOpenDialog({ title: 'Choose Stockfish executable', properties: ['openFile'] })
    return result.canceled ? null : result.filePaths[0]
  })
  ipcMain.handle('engine:install', () => installStockfish())
  ipcMain.handle('engine:bestmove', (_event, moves: string[], level: 'low' | 'medium' | 'high', path?: string) => bestMove(moves, level, path))
  ipcMain.handle('online:start', (_event, options: { minutes: number; increment: number; color: string; target?: string }) => online.start(options))
  ipcMain.handle('online:resume', () => online.resume())
  ipcMain.handle('online:cancel', () => online.cancel())
  ipcMain.handle('online:move', (_event, id: string, move: string) => online.move(id, move))
  ipcMain.handle('online:action', (_event, id: string, action: 'resign' | 'abort' | 'takeback' | 'declineTakeback') => online.action(id, action))
  createWindow()
  app.on('activate', () => { if (BrowserWindow.getAllWindows().length === 0) createWindow() })
})

app.on('window-all-closed', () => { online.cancel(); if (process.platform !== 'darwin') app.quit() })
