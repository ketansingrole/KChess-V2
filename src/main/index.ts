import { app, BrowserWindow, dialog, Menu, nativeImage, nativeTheme, shell } from 'electron'
import { handleAppProtocol, registerAppScheme } from './appProtocol'
import { microphoneAccess, openMicrophoneSettings, setupMediaPermissions } from './microphone'
import { handle } from './ipc'
import { IPC_EVENTS, type IpcEvents } from '../shared/ipc'
import { join } from 'node:path'
import { mkdirSync } from 'node:fs'
import { setupDiagnostics, exportDiagnostics } from './diagnostics'
import { bestMove, engineStatus, isTrustedEnginePath, stopEngine, trustEnginePath } from './engine'
import { closeDb } from './db'
import { notify } from './notify'
import { loadCustomThemes, themesDir } from './themes'
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
  puzzleActivity,
  puzzleDaily,
  puzzleDashboard,
  puzzleNext,
  puzzleSolve,
  ratingHistory,
  stormDashboard,
  syncGames,
} from './lichess'
import {
  cancelPuzzleDb,
  deletePuzzleDb,
  installPuzzleDb,
  localLadder,
  localPuzzles,
  puzzleDbStatus,
} from './puzzleDb'
import { clearRuns, runSummary, saveRun } from './runs'
import {
  clearVoiceHistory,
  exportVoiceHistory,
  saveVoiceAttempt,
  updateVoiceAttempt,
  voiceHistory,
} from './voiceLog'
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
  assertActivityMax,
  assertBestMoveOptions,
  assertDays,
  assertFriendList,
  assertGameId,
  assertLadderQuery,
  assertLevel,
  assertLocalQuery,
  assertMoves,
  assertNotification,
  assertOnlineOptions,
  assertPuzzleRequest,
  assertPuzzleSolve,
  assertRunInput,
  assertRunKind,
  assertUsernames,
  assertSettings,
  assertUci,
  assertUsername,
  assertVoiceAttempt,
  assertVoiceId,
  assertVoiceLimit,
  assertVoiceUpdate,
} from '../shared/validate'

let window: BrowserWindow | null = null
registerAppScheme()
app.setName('KChess')
// An explicit profile is useful for isolated tests and development with fresh data.
if (process.env.KCHESS_USER_DATA_DIR) {
  mkdirSync(process.env.KCHESS_USER_DATA_DIR, { recursive: true })
  app.setPath('userData', process.env.KCHESS_USER_DATA_DIR)
  app.setPath('sessionData', process.env.KCHESS_USER_DATA_DIR)
}

/** Send to the renderer unless the window is gone (e.g. closed on macOS while a stream is live). */
function send<K extends keyof IpcEvents>(channel: K, payload: IpcEvents[K]): void {
  if (window && !window.isDestroyed()) window.webContents.send(channel, payload)
}
const online = new OnlineSession(
  (event: OnlineEvent) => send(IPC_EVENTS.online, event),
  (message: string) => send(IPC_EVENTS.error, message),
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
  const appOrigin = process.env.KCHESS_NUXT_URL || 'kchess://app/'
  window.webContents.on('will-navigate', (event, url) => {
    if (!url.startsWith(appOrigin)) event.preventDefault()
  })
  window.webContents.on('render-process-gone', (_event, details) => {
    console.error('Renderer exited:', details.reason, details.exitCode)
  })
  window.webContents.on('preload-error', (_event, _path, error) => {
    console.error('Preload failed:', error)
  })
  window.webContents.on('console-message', (_event, level, message) => {
    if (level >= 2) console.error('Renderer:', message)
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
  else void window.loadURL('kchess://app/index.html').catch(console.error)
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

void app
  .whenReady()
  .then(() => {
    handleAppProtocol()
    setupMediaPermissions((url) => url.startsWith(process.env.KCHESS_NUXT_URL || 'kchess://app/'))
    // Themed app icon (Dock / Mission Control / Cmd-Tab), live-updated when
    // the system appearance changes. Packaged builds fall back to the bundle
    // .icns until this override applies.
    setupDiagnostics()
    handle('exportDiagnostics', exportDiagnostics)
    refreshAppIcon()
    nativeTheme.on('updated', refreshAppIcon)
    setupAppMenu()
    handle('loadData', () => loadData())
    handle('saveSettings', async (_event, raw: unknown) => {
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
    handle('addAccount', (_event, username: unknown) => addAccount(assertUsername(username)))
    handle('removeAccount', (_event, username: unknown) => removeAccount(assertUsername(username)))
    handle('syncGames', (_event, username?: unknown) =>
      syncGames(username === undefined ? undefined : assertUsername(username)),
    )
    handle('gamePgn', (_event, account: unknown, id: unknown) => {
      return gamePgn(assertUsername(account), assertGameId(id))
    })
    handle('cachedProfile', (_event, username: unknown) => cachedProfile(assertUsername(username)))
    handle('profile', (_event, username: unknown) => profile(assertUsername(username)))
    handle('ratingHistory', (_event, username: unknown) => ratingHistory(assertUsername(username)))
    handle('connectLichess', () => connectLichess())
    handle('engineStatus', async () => engineStatus((await getSettings()).enginePath))
    handle('chooseEngine', async () => {
      const result = await dialog.showOpenDialog({
        title: 'Choose Stockfish executable',
        properties: ['openFile'],
      })
      const path = result.canceled ? null : (result.filePaths[0] ?? null)
      if (path) trustEnginePath(path)
      return path
    })
    handle('installEngine', async () => {
      if (process.platform !== 'darwin')
        throw new Error('Downloading Stockfish currently supports macOS.')
      const { path, version, updated } = await installManagedEngine()
      return { path, version, updated }
    })
    handle('deleteEngine', async () => {
      stopEngine()
      await deleteManagedEngine()
    })
    handle('bestMove', async (_event, moves: unknown, level: unknown, options?: unknown) =>
      bestMove(
        assertMoves(moves),
        assertLevel(level),
        (await getSettings()).enginePath,
        assertBestMoveOptions(options),
      ),
    )
    handle('startOnline', (_event, options: unknown) => online.start(assertOnlineOptions(options)))
    handle('resumeOnline', () => online.resume())
    handle('cancelOnline', () => online.cancel())
    handle('playOnline', (_event, id: unknown, move: unknown) =>
      online.move(assertGameId(id), assertUci(move)),
    )
    handle('onlineAction', (_event, id: unknown, action: unknown) =>
      online.action(assertGameId(id), assertAction(action)),
    )
    handle('presence', (_event, usernames: unknown) => online.presence(assertUsernames(usernames)))
    handle('clearAccountData', async (_event, username: unknown) => {
      const name = assertUsername(username)
      const data = await clearAccountData(name)
      forgetProfile(name)
      return data
    })
    handle('following', () => followedUsers())
    handle('addFriends', async (_event, usernames: unknown) => {
      const names = assertFriendList(usernames)
      const data = await addFriends(names)
      await primeProfiles(names)
      return data
    })
    handle('notify', (_event, request: unknown) =>
      notify(window, assertNotification(request), (alert) => send(IPC_EVENTS.notification, alert)),
    )
    handle('loadThemes', () => loadCustomThemes())
    handle('openThemesFolder', async () => {
      await loadCustomThemes() // makes sure the folder exists
      await shell.openPath(themesDir())
    })
    handle('openNotificationSettings', async () => {
      // Fixed URLs only: nothing from the renderer reaches openExternal.
      const url =
        process.platform === 'darwin'
          ? 'x-apple.systempreferences:com.apple.Notifications-Settings.extension'
          : process.platform === 'win32'
            ? 'ms-settings:notifications'
            : undefined
      if (!url) return false
      await shell.openExternal(url)
      return true
    })
    handle('microphoneAccess', (_event, request: unknown) => microphoneAccess(request === true))
    handle('openMicrophoneSettings', () => openMicrophoneSettings())
    handle('puzzleNext', (_event, request: unknown) => puzzleNext(assertPuzzleRequest(request)))
    handle('puzzleSolve', (_event, request: unknown) => puzzleSolve(assertPuzzleSolve(request)))
    handle('puzzleDaily', () => puzzleDaily())
    handle('puzzleDashboard', (_event, account: unknown, days: unknown) =>
      puzzleDashboard(assertUsername(account), assertDays(days)),
    )
    handle('puzzleActivity', (_event, account: unknown, max: unknown) =>
      puzzleActivity(assertUsername(account), assertActivityMax(max)),
    )
    handle('stormDashboard', (_event, username: unknown, days: unknown) =>
      stormDashboard(assertUsername(username), assertDays(days)),
    )
    handle('puzzleDbStatus', () => puzzleDbStatus())
    handle('puzzleDbInstall', () =>
      installPuzzleDb((progress) => send(IPC_EVENTS.puzzleProgress, progress)),
    )
    handle('puzzleDbCancel', () => cancelPuzzleDb())
    handle('puzzleDbDelete', () => deletePuzzleDb())
    handle('localPuzzles', (_event, query: unknown) => localPuzzles(assertLocalQuery(query)))
    handle('localLadder', (_event, query: unknown) => localLadder(assertLadderQuery(query)))
    handle('saveRun', (_event, run: unknown) => saveRun(assertRunInput(run)))
    handle('runSummary', (_event, kind: unknown) => runSummary(assertRunKind(kind)))
    handle('clearRuns', (_event, kind: unknown) =>
      clearRuns(kind === undefined ? undefined : assertRunKind(kind)),
    )
    handle('saveVoiceAttempt', (_event, attempt: unknown) =>
      saveVoiceAttempt(assertVoiceAttempt(attempt)),
    )
    handle('updateVoiceAttempt', (_event, id: unknown, update: unknown) =>
      updateVoiceAttempt(assertVoiceId(id), assertVoiceUpdate(update)),
    )
    handle('voiceHistory', (_event, limit: unknown) => voiceHistory(assertVoiceLimit(limit)))
    handle('clearVoiceHistory', () => clearVoiceHistory())
    handle('exportVoiceHistory', () => exportVoiceHistory())
    handle('usage', () => usageReport())
    handle('resetUsage', () => resetUsage())
    createWindow()
    app.on('activate', () => {
      if (BrowserWindow.getAllWindows().length === 0) createWindow()
    })
  })
  .catch((error: unknown) => {
    console.error('Startup failed:', error)
    app.quit()
  })

app.on('window-all-closed', () => {
  cancelPuzzleDb()
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
