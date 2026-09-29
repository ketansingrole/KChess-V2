import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  Menu,
  nativeImage,
  nativeTheme,
  shell,
} from 'electron'
import { join } from 'node:path'
import { bestMove, engineStatus, isTrustedEnginePath, stopEngine, trustEnginePath } from './engine'
import { closeDb } from './db'
import { flushUsage, resetUsage, usageReport } from './usage'
import { deleteManagedEngine, installManagedEngine } from './managedEngine'
import {
  OnlineSession,
  cachedProfile,
  connectLichess,
  followedUsers,
  forgetProfile,
  primeProfiles,
  profile,
  ratingHistory,
  syncGames,
} from './lichess'
import {
  addAccount,
  addFriends,
  clearAccountData,
  gamePgn,
  getSettings,
  loadData,
  removeAccount,
  saveSettings,
} from './store'
import type { OnlineEvent } from '../shared/types'
import {
  assertAction,
  assertFriendList,
  assertGameId,
  assertLevel,
  assertMoves,
  assertOnlineOptions,
  assertUsernames,
  assertSettings,
  assertUci,
  assertUsername,
} from '../shared/validate'

let window: BrowserWindow | null = null
app.setName('KChess')

/** Send to the renderer unless the window is gone (e.g. closed on macOS while a stream is live). */
function send(channel: string, payload: unknown): void {
  if (window && !window.isDestroyed()) window.webContents.send(channel, payload)
}
const online = new OnlineSession(
  (event: OnlineEvent) => send('online:event', event),
  (message: string) => send('online:error', message),
)

function appIconPath(): string | undefined {
  // Dev: load from project build dir.
  // Built app: Nuxt's generated public assets are bundled with the app.
  // Packaged .app: bundle .icns is used automatically; setting dock icon is harmless.
  try {
    if (process.env.KCHESS_NUXT_URL) return join(app.getAppPath(), 'build', 'icon.png')
    return join(app.getAppPath(), '.output/public/icon.png')
  } catch {
    return undefined
  }
}

function windowIcon(): string | undefined {
  return appIconPath()
}

// Themed Dock / Cmd-Tab icon on macOS: warm gold tile in light appearance,
// deep bronze tile in dark appearance. Setting the application icon at
// runtime updates the Dock, Cmd-Tab switcher, and Mission Control together.
function themeIconPath(dark: boolean): string | undefined {
  const file = dark ? 'icon-dark.png' : 'icon-light.png'
  try {
    if (process.env.KCHESS_NUXT_URL) return join(app.getAppPath(), 'build', file)
    return join(app.getAppPath(), '.output/public', file)
  } catch {
    return undefined
  }
}

function refreshAppIcon(): void {
  if (process.platform !== 'darwin') return
  try {
    const iconPath = themeIconPath(nativeTheme.shouldUseDarkColors)
    if (iconPath) {
      const dockIcon = nativeImage.createFromPath(iconPath)
      if (!dockIcon.isEmpty()) app.dock?.setIcon(dockIcon)
    }
  } catch {
    /* dock icon is optional */
  }
}

function createWindow(): void {
  const icon = windowIcon()
  window = new BrowserWindow({
    title: 'KChess',
    width: 1320,
    height: 880,
    minWidth: 640,
    minHeight: 620,
    backgroundColor: '#111827',
    icon,
    // macOS: traffic lights (close/minimize/maximize) float over the sidebar
    // header, vertically centered on the 52px header row so they share one
    // center line with the sidebar toggle and the topbar controls.
    ...(process.platform === 'darwin'
      ? { titleBarStyle: 'hiddenInset' as const, trafficLightPosition: { x: 12, y: 20 } }
      : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  })
  window.on('closed', () => {
    window = null
  })
  // The app only ever shows its own UI: never let the window navigate elsewhere.
  const appOrigin = process.env.KCHESS_NUXT_URL ?? 'file://'
  window.webContents.on('will-navigate', (event, url) => {
    if (!url.startsWith(appOrigin)) event.preventDefault()
  })
  window.webContents.on('did-fail-load', (_event, code, description, url) =>
    console.error('Renderer load failed:', code, description, url),
  )
  window.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith('https://lichess.org/')) void shell.openExternal(url)
    return { action: 'deny' }
  })
  if (process.env.KCHESS_NUXT_URL)
    void window.loadURL(process.env.KCHESS_NUXT_URL).catch(console.error)
  else
    void window.loadFile(join(app.getAppPath(), '.output/public/index.html')).catch(console.error)
}

function setupAppMenu(): void {
  // macOS: label the application menu with the app name (About KChess,
  // Hide KChess, Quit KChess, …) instead of leaving Electron defaults.
  if (process.platform !== 'darwin') return
  Menu.setApplicationMenu(
    Menu.buildFromTemplate([
      {
        label: app.name,
        submenu: [
          { role: 'about' },
          { type: 'separator' },
          { role: 'services' },
          { type: 'separator' },
          { role: 'hide' },
          { role: 'hideOthers' },
          { role: 'unhide' },
          { type: 'separator' },
          { role: 'quit' },
        ],
      },
      { role: 'editMenu' },
      { role: 'fileMenu' },
      { role: 'viewMenu' },
      { role: 'windowMenu' },
    ]),
  )
}

app.whenReady().then(() => {
  // Themed app icon (Dock / Mission Control / Cmd-Tab), live-updated when
  // the system appearance changes. Packaged builds fall back to the bundle
  // .icns until this override applies.
  refreshAppIcon()
  nativeTheme.on('updated', refreshAppIcon)
  setupAppMenu()
  ipcMain.handle('data:load', () => loadData())
  ipcMain.handle('settings:save', async (_event, raw: unknown) => {
    const settings = assertSettings(raw)
    // The renderer may not point the app at an arbitrary executable: only a path
    // picked in the native dialog, downloaded by KChess, or already saved is accepted.
    if (settings.enginePath && !isTrustedEnginePath(settings.enginePath)) {
      const stored = await getSettings()
      if (settings.enginePath !== stored.enginePath)
        throw new Error('Choose the Stockfish executable with the file picker.')
    }
    return saveSettings(settings)
  })
  ipcMain.handle('account:add', (_event, username: unknown) => addAccount(assertUsername(username)))
  ipcMain.handle('account:remove', (_event, username: unknown) =>
    removeAccount(assertUsername(username)),
  )
  ipcMain.handle('games:sync', (_event, username?: unknown) =>
    syncGames(username === undefined ? undefined : assertUsername(username)),
  )
  ipcMain.handle('games:pgn', (_event, account: unknown, id: unknown) => {
    return gamePgn(assertUsername(account), assertGameId(id))
  })
  ipcMain.handle('lichess:cached-profile', (_event, username: unknown) =>
    cachedProfile(assertUsername(username)),
  )
  ipcMain.handle('lichess:profile', (_event, username: unknown) =>
    profile(assertUsername(username)),
  )
  ipcMain.handle('lichess:rating', (_event, username: unknown) =>
    ratingHistory(assertUsername(username)),
  )
  ipcMain.handle('lichess:connect', () => connectLichess())
  ipcMain.handle('engine:status', async () => engineStatus((await getSettings()).enginePath))
  ipcMain.handle('engine:choose', async () => {
    const result = await dialog.showOpenDialog({
      title: 'Choose Stockfish executable',
      properties: ['openFile'],
    })
    const path = result.canceled ? null : (result.filePaths[0] ?? null)
    if (path) trustEnginePath(path)
    return path
  })
  ipcMain.handle('engine:install', async () => {
    if (process.platform !== 'darwin')
      throw new Error('Downloading Stockfish currently supports macOS.')
    const { path, version, updated } = await installManagedEngine()
    return { path, version, updated }
  })
  ipcMain.handle('engine:delete', async () => {
    stopEngine()
    await deleteManagedEngine()
  })
  ipcMain.handle('engine:bestmove', async (_event, moves: unknown, level: unknown) =>
    bestMove(assertMoves(moves), assertLevel(level), (await getSettings()).enginePath),
  )
  ipcMain.handle('online:start', (_event, options: unknown) =>
    online.start(assertOnlineOptions(options)),
  )
  ipcMain.handle('online:resume', () => online.resume())
  ipcMain.handle('online:cancel', () => online.cancel())
  ipcMain.handle('online:move', (_event, id: unknown, move: unknown) =>
    online.move(assertGameId(id), assertUci(move)),
  )
  ipcMain.handle('online:action', (_event, id: unknown, action: unknown) =>
    online.action(assertGameId(id), assertAction(action)),
  )
  ipcMain.handle('online:presence', (_event, usernames: unknown) =>
    online.presence(assertUsernames(usernames)),
  )
  ipcMain.handle('account:clear-data', async (_event, username: unknown) => {
    const name = assertUsername(username)
    const data = await clearAccountData(name)
    forgetProfile(name)
    return data
  })
  ipcMain.handle('friends:following', () => followedUsers())
  ipcMain.handle('friends:add-many', async (_event, usernames: unknown) => {
    const names = assertFriendList(usernames)
    const data = await addFriends(names)
    await primeProfiles(names)
    return data
  })
  ipcMain.handle('usage:report', () => usageReport())
  ipcMain.handle('usage:reset', () => resetUsage())
  createWindow()
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('window-all-closed', () => {
  online.cancel()
  stopEngine()
  if (process.platform !== 'darwin') app.quit()
})
app.on('will-quit', () => {
  online.cancel()
  stopEngine()
  flushUsage()
  closeDb()
})
