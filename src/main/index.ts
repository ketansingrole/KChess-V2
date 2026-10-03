import {
  app,
  BrowserWindow,
  dialog,
  Menu,
  nativeImage,
  nativeTheme,
  powerMonitor,
  shell,
} from 'electron'
import { handleAppProtocol, registerAppScheme } from './appProtocol'
import { VoiceModelCache } from './voiceModel'
import { microphoneAccess, openMicrophoneSettings, setupMediaPermissions } from './microphone'
import { handle, setIpcOwner } from './ipc'
import { isAppUrl } from './appOrigin'
import { IPC_EVENTS, type IpcEvents } from '../shared/ipc'
import { join } from 'node:path'
import { mkdirSync } from 'node:fs'
import { setupAppUpdates, APP_RELEASES_URL } from './setupAppUpdates'
import type { AppUpdates } from './appUpdates'
import { setupDiagnostics, exportDiagnostics } from './diagnostics'
import { configureEngineResources } from './engineScheduler'
import { recordTiming } from './performance'
import { positionLookups } from './setupPositionLookup'
import {
  bestMove,
  computerPlaying,
  engineStatus,
  engineIdentity,
  isTrustedEnginePath,
  stopEngine,
  trustEnginePath,
} from './engine'
import { closeDb } from './db'
import { notify } from './notify'
import { loadCustomThemes, themesDir } from './themes'
import { flushUsage, resetUsage, usageReport } from './usage'
import { deleteManagedEngine, installManagedEngine } from './managedEngine'
import { analysisRunning, startAnalysis, stopAnalysis } from './analysis'
import {
  cancelReview,
  getReview,
  requestReview,
  restartReviewEngine,
  reviewStatus,
  reviewsChanged,
  setupReviews,
  stopReviews,
} from './review'
import { reviewSummaries } from './reviewStore'
import {
  OnlineSession,
  cachedProfile,
  connectLichess,
  fetchLichessReviews,
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
  closePuzzleWorker,
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
  gamePage,
  gameLibraryOverview,
  gameRatingHistory,
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
  assertGamePageQuery,
  assertLadderQuery,
  assertLevel,
  assertLocalQuery,
  assertMoves,
  assertNotification,
  assertOnlineOptions,
  assertPuzzleRequest,
  assertPuzzleSolve,
  assertReviewKey,
  assertReviewRequest,
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
let appUpdates: AppUpdates | undefined
setIpcOwner(() => window)
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
  (state) => {
    if (state.gameId && state.phase !== 'idle') {
      stopEngine()
      stopAnalysis()
      restartReviewEngine()
    }
    send(IPC_EVENTS.onlineState, state)
  },
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
  // macOS keeps its traffic lights (hiddenInset). Windows/Linux use OpenChamber-style
  // frameless chrome: no OS title bar at all, the renderer draws its own
  // minimize/maximize/close buttons in the topbar. The menu bar stays available
  // via Alt but is hidden so the window doesn't look like a legacy Win32 app.
  const frameless = process.platform === 'win32' || process.platform === 'linux'
  window = new BrowserWindow({
    title: 'KChess',
    width: 1320,
    height: 880,
    minWidth: 640,
    minHeight: 620,
    backgroundColor: '#111827',
    icon,
    frame: frameless ? false : undefined,
    autoHideMenuBar: process.platform !== 'darwin',
    titleBarStyle:
      process.platform === 'darwin'
        ? ('hiddenInset' as const)
        : frameless
          ? ('hidden' as const)
          : undefined,
    // macOS: traffic lights (close/minimize/maximize) float over the sidebar
    // header, vertically centered on the 52px header row so they share one
    // center line with the sidebar toggle and the topbar controls.
    trafficLightPosition: process.platform === 'darwin' ? { x: 12, y: 20 } : undefined,
    // Frameless windows draw their own controls; never show the overlay ones.
    titleBarOverlay: frameless ? false : undefined,
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
  window.on('maximize', () => {
    if (window && !window.isDestroyed())
      window.webContents.send(IPC_EVENTS.windowMaximized, { maximized: true })
  })
  window.on('unmaximize', () => {
    if (window && !window.isDestroyed())
      window.webContents.send(IPC_EVENTS.windowMaximized, { maximized: false })
  })
  // The app only ever shows its own UI: never let the window navigate elsewhere.
  window.webContents.on('will-navigate', (event, url) => {
    if (!isAppUrl(url)) event.preventDefault()
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
  .then(async () => {
    const voiceModel = new VoiceModelCache(join(app.getPath('userData'), 'voice'))
    handleAppProtocol(voiceModel.path)
    handle('ensureVoiceModel', async (event) => {
      await voiceModel.ensure((progress) => {
        if (!event.sender.isDestroyed()) event.sender.send(IPC_EVENTS.voiceModelProgress, progress)
      })
      return 'kchess://app/voice/model.tar.gz'
    })
    setupMediaPermissions(isAppUrl)
    // Themed app icon (Dock / Mission Control / Cmd-Tab), live-updated when
    // the system appearance changes. Packaged builds fall back to the bundle
    // .icns until this override applies.
    setupDiagnostics()
    configureEngineResources(() => powerMonitor.isOnBatteryPower())
    handle('exportDiagnostics', exportDiagnostics)
    handle('recordPerformance', (_event, name: unknown, milliseconds: unknown) => {
      if (
        !['app.ready', 'board.frame', 'voice.activation'].includes(String(name)) ||
        typeof milliseconds !== 'number' ||
        !Number.isFinite(milliseconds) ||
        milliseconds < 0 ||
        milliseconds > 300_000
      )
        throw new Error('Invalid timing sample.')
      recordTiming(String(name), milliseconds)
    })
    handle('windowMinimize', () => {
      if (window && !window.isDestroyed()) window.minimize()
    })
    handle('windowToggleMaximize', () => {
      if (window && !window.isDestroyed()) {
        if (window.isMaximized()) window.unmaximize()
        else window.maximize()
        return { maximized: window.isMaximized() }
      }
      return { maximized: false }
    })
    handle('windowClose', () => {
      if (window && !window.isDestroyed()) window.close()
    })
    handle('windowIsMaximized', () =>
      Boolean(window && !window.isDestroyed() && window.isMaximized()),
    )
    refreshAppIcon()
    nativeTheme.on('updated', refreshAppIcon)
    setupAppMenu()
    appUpdates = await setupAppUpdates(
      (status) => send(IPC_EVENTS.appUpdate, status),
      () => {
        // Closing the last window can initiate quit before an installer fails (e.g. elevation
        // is cancelled). Restore the UI so a prevented quit never leaves a hidden process.
        if (!window || window.isDestroyed()) createWindow()
        else {
          if (window.isMinimized()) window.restore()
          window.show()
          window.focus()
        }
      },
    )
    handle('appUpdateStatus', () => appUpdates!.status())
    handle('checkAppUpdate', () => appUpdates!.check())
    handle('downloadAppUpdate', () => appUpdates!.download())
    handle('installAppUpdate', () => appUpdates!.install())
    handle('openAppReleases', () => shell.openExternal(APP_RELEASES_URL))
    handle('loadData', () => loadData())
    handle('saveSettings', async (_event, raw: unknown) => {
      const settings = assertSettings(raw)
      const previous = await getSettings()
      // The renderer may not point the app at an arbitrary executable: only a path
      // picked in the native dialog, downloaded by KChess, or already saved is accepted.
      if (settings.enginePath && !isTrustedEnginePath(settings.enginePath)) {
        const stored = await getSettings()
        if (settings.enginePath !== stored.enginePath)
          throw new Error('Choose the Stockfish executable with the file picker.')
      }
      const saved = await saveSettings(settings)
      if (previous.enginePath !== saved.enginePath) {
        stopEngine(true)
        stopAnalysis(true)
        restartReviewEngine()
      }
      appUpdates!.applyPreferences(saved)
      reviewsChanged()
      return saved
    })
    handle('addAccount', (_event, username: unknown) => addAccount(assertUsername(username)))
    handle('removeAccount', (_event, username: unknown) => removeAccount(assertUsername(username)))
    handle('syncGames', async (_event, username?: unknown) => {
      const data = await syncGames(username === undefined ? undefined : assertUsername(username))
      reviewsChanged()
      return data
    })
    handle('gamePage', (_event, query: unknown) => gamePage(assertGamePageQuery(query)))
    handle('gameLibraryOverview', () => gameLibraryOverview())
    handle('gameRatingHistory', (_event, account: unknown) =>
      gameRatingHistory(assertUsername(account)),
    )
    handle('gamePgn', (_event, account: unknown, id: unknown) => {
      return gamePgn(assertUsername(account), assertGameId(id))
    })
    handle('cachedProfile', (_event, username: unknown) => cachedProfile(assertUsername(username)))
    handle('profile', (_event, username: unknown) => profile(assertUsername(username)))
    handle('ratingHistory', (_event, username: unknown) => ratingHistory(assertUsername(username)))
    handle('connectLichess', () => connectLichess())
    handle('engineStatus', async () => {
      const status = await engineStatus((await getSettings()).enginePath)
      return { ...status, identity: status.ready ? await engineIdentity(status) : undefined }
    })
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
      const { path, version, updated } = await installManagedEngine()
      return { path, version, updated }
    })
    handle('deleteEngine', async () => {
      stopEngine(true)
      stopAnalysis(true)
      restartReviewEngine()
      await deleteManagedEngine()
    })
    handle('stopEngine', () => stopEngine())
    handle('bestMove', async (_event, moves: unknown, level: unknown, options?: unknown) => {
      if (online.playing)
        throw new Error('Engine assistance is unavailable during a live Lichess game.')
      return bestMove(
        assertMoves(moves),
        assertLevel(level),
        (await getSettings()).enginePath,
        assertBestMoveOptions(options),
      )
    })
    handle('startAnalysis', async (_event, request: unknown) => {
      if (online.playing) throw new Error('Analysis is unavailable during a live Lichess game.')
      return startAnalysis(request, (await getSettings()).enginePath, (update) =>
        send(IPC_EVENTS.analysis, update),
      )
    })
    handle('positionLookup', async (_event, kind: unknown, fen: unknown) => {
      if (online.playing)
        throw new Error('Position lookups are unavailable during a live Lichess game.')
      return positionLookups.lookup(kind, fen)
    })
    handle('stopAnalysis', () => stopAnalysis())
    handle('reviewGet', (_event, fen: unknown, moves: unknown) => {
      const request = assertReviewRequest({ fen, moves })
      return getReview(request.fen, request.moves)
    })
    handle('reviewRequest', (_event, request: unknown) => {
      if (online.playing) throw new Error('Review is unavailable during a live Lichess game.')
      return requestReview(assertReviewRequest(request))
    })
    handle('reviewCancel', (_event, key: unknown) => cancelReview(assertReviewKey(key)))
    handle('reviewStatus', () => reviewStatus())
    handle('reviewSummaries', () => reviewSummaries())
    setupReviews({
      settings: getSettings,
      accounts: async () => {
        const { accounts } = await loadData()
        // Your own accounts: the connected ones, or the first one added when none is.
        const own = accounts.filter((account) => account.connected)
        return (own.length ? own : accounts.slice(0, 1)).map((account) => account.username)
      },
      onBattery: () => powerMonitor.isOnBatteryPower(),
      // The computer opponent counts as busy for a minute after its move: the game goes on.
      busy: () =>
        online.playing
          ? 'online'
          : analysisRunning() || computerPlaying(60_000)
            ? 'engine'
            : undefined,
      fetchLichess: fetchLichessReviews,
      update: (update) => send(IPC_EVENTS.reviewUpdate, update),
      status: (status) => send(IPC_EVENTS.reviewStatus, status),
    })
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
    appUpdates.start()
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
  stopEngine(true)
  stopAnalysis(true)
  if (process.platform !== 'darwin') app.quit()
})
app.on('before-quit', (event) => {
  if (appUpdates?.shouldInstallOnQuit()) {
    // Squirrel.Mac may need to finish staging the verified ZIP before it can quit.
    event.preventDefault()
    try {
      appUpdates.install(false)
    } catch (cause) {
      // The updater reports the failure in Settings; leave the app open so it can be retried.
      console.warn('Could not install on quit:', cause)
    }
  }
})
app.on('will-quit', () => {
  appUpdates?.stop()
  online.cancel()
  stopEngine(true)
  stopAnalysis(true)
  stopReviews()
  flushUsage()
  closePuzzleWorker()
  closeDb()
})
