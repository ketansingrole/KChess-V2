import { PERFORMANCE_NAMES, type PerformanceName } from '../shared/rendererDiagnostics'
import { app, BrowserWindow, dialog, Menu, nativeImage, nativeTheme, shell } from 'electron'
import { handleAppProtocol, registerAppScheme } from './appProtocol'
import { VoiceModelCache } from './voiceModel'
import { microphoneAccess, openMicrophoneSettings, setupMediaPermissions } from './microphone'
import { handle, setIpcOwner, assertIpcComplete } from './ipc'
import { isAppUrl } from './appOrigin'
import {
  FORWARDED_CORE_EVENTS,
  IPC_EVENTS,
  type IpcArguments,
  type IpcEvents,
  type IpcResult,
} from '../shared/ipc'
import { join } from 'node:path'
import { mkdirSync } from 'node:fs'
import { setupAppUpdates, APP_RELEASES_URL } from './setupAppUpdates'
import type { AppUpdates } from './appUpdates'
import { setupDiagnostics, exportDiagnostics } from './diagnostics'
import { notify } from './notify'
import { loadCustomThemes, themesDir } from './themes'
import {
  createKChessCore,
  CORE_METHODS,
  logDebug,
  logError,
  logWarn,
  recordTiming,
  type CoreMethod,
  type KChessCore,
} from '../core'
import { electronPlatform } from './platform'
import { saveWithDialog } from './saveDialog'
import { assertExport, assertNotification } from '../shared/validate'
import type { ChallengeInfo } from '../shared/types'

let window: BrowserWindow | null = null
let appUpdates: AppUpdates | undefined
let core: KChessCore | undefined
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

/** Bring the window forward, e.g. when the browser sign-in returns. */
function bringToFront(): void {
  if (!window || window.isDestroyed()) return
  if (window.isMinimized()) window.restore()
  window.show()
  // macOS keeps the browser active unless the app explicitly takes focus.
  app.focus({ steal: true })
  window.focus()
}

function challengeAlert(service: KChessCore, challenge: ChallengeInfo): void {
  const control =
    challenge.timeControl.type === 'clock'
      ? `${challenge.timeControl.limit / 60}+${challenge.timeControl.increment}`
      : challenge.timeControl.type === 'correspondence'
        ? `${challenge.timeControl.days} days per move`
        : 'no clock'
  void notify(
    window,
    {
      kind: 'challenge',
      title: challenge.rematchOf
        ? `${challenge.opponent.name} wants a rematch`
        : `${challenge.opponent.name} challenges you`,
      body: `${challenge.variantName} · ${control} · ${challenge.rated ? 'Rated' : 'Casual'} (@${challenge.account})`,
    },
    (alert) => send(IPC_EVENTS.notification, alert),
    service.settings,
  ).catch((cause: unknown) => {
    logDebug('notify', 'Challenge notification failed:', cause)
    return undefined
  })
}

/** Every CoreApi method is served to the renderer as-is; the core validates its own input. */
function forwardCore(service: KChessCore): void {
  for (const method of CORE_METHODS) {
    // IPC contracts have already checked the argument shapes against DesktopApi.
    const call = service[method] as (...args: unknown[]) => IpcResult<CoreMethod>
    handle(method, (_event, ...args) => call(...(args as IpcArguments<CoreMethod>)) as never)
  }
  for (const event of FORWARDED_CORE_EVENTS)
    service.on(event, (payload) => send(event, payload as never))
  service.on('challenge:received', (challenge) => challengeAlert(service, challenge))
  service.on('settings:saved', (settings) => appUpdates?.applyPreferences(settings))
}

function appIconPath(): string | undefined {
  // Dev: load from project build dir.
  // Built app: Nuxt's generated public assets are bundled with the app.
  // Packaged .app: bundle .icns is used automatically; setting dock icon is harmless.
  try {
    if (process.env.KCHESS_NUXT_URL) return join(app.getAppPath(), 'build', 'icon.png')
    return join(app.getAppPath(), '.output/public/icon.png')
  } catch (cause) {
    logDebug('startup', 'App icon path is unavailable:', cause)
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
  } catch (cause) {
    logDebug('startup', 'Theme icon path is unavailable:', cause)
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
  } catch (cause) {
    logDebug('startup', 'Dock icon refresh failed:', cause)
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
      // `import.meta.dirname`, not `__dirname`: electron-vite's CommonJS shim is placed after the
      // last `import` it finds, which can be an example inside a bundled library's doc comment.
      preload: join(import.meta.dirname, '../preload/index.cjs'),
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
    logError('renderer', 'Renderer exited:', details.reason, details.exitCode)
  })
  window.webContents.on('preload-error', (_event, _path, error) => {
    logError('renderer', 'Preload failed:', error)
  })
  window.webContents.on('console-message', ({ level, message }) => {
    if (level === 'warning' || level === 'error') logError('renderer', 'Renderer console:', message)
  })
  window.webContents.on('did-fail-load', (_event, code, description, url) =>
    logError('renderer', 'Renderer load failed:', code, description, url),
  )
  window.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith('https://lichess.org/')) void shell.openExternal(url)
    return { action: 'deny' }
  })
  if (process.env.KCHESS_NUXT_URL)
    void window.loadURL(process.env.KCHESS_NUXT_URL).catch((error: unknown) => {
      logError('renderer', 'Renderer loadURL failed:', process.env.KCHESS_NUXT_URL, error)
    })
  else
    void window.loadURL('kchess://app/index.html').catch((error: unknown) => {
      logError('renderer', 'Renderer loadURL failed:', 'kchess://app/index.html', error)
    })
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
    handle('voiceModelStatus', () => voiceModel.status())
    handle('ensureVoiceModel', async (event) => {
      await voiceModel.ensure((progress) => {
        if (!event.sender.isDestroyed()) event.sender.send(IPC_EVENTS.voiceModelProgress, progress)
      })
      return 'kchess://app/voice/model.tar.gz'
    })
    setupMediaPermissions(isAppUrl)
    setupDiagnostics()
    const service = createKChessCore(electronPlatform(bringToFront))
    core = service
    forwardCore(service)
    handle('exportDiagnostics', exportDiagnostics)
    handle('reportRendererError', (_event, report) => {
      logError('renderer', 'Renderer page error:', report)
    })
    handle('recordPerformance', (_event, name: unknown, milliseconds: unknown) => {
      if (
        !PERFORMANCE_NAMES.includes(name as PerformanceName) ||
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
    // Themed app icon (Dock / Mission Control / Cmd-Tab), live-updated when
    // the system appearance changes. Packaged builds fall back to the bundle
    // .icns until this override applies.
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
      service.settings,
    )
    handle('appUpdateStatus', () => appUpdates!.status())
    handle('checkAppUpdate', () => appUpdates!.check())
    handle('downloadAppUpdate', () => appUpdates!.download())
    handle('installAppUpdate', () => appUpdates!.install())
    handle('openAppReleases', () => shell.openExternal(APP_RELEASES_URL))
    handle('chooseEngine', async () => {
      const result = await dialog.showOpenDialog({
        title: 'Choose Stockfish executable',
        properties: ['openFile'],
      })
      const path = result.canceled ? null : (result.filePaths[0] ?? null)
      if (path) service.trustEnginePath(path)
      return path
    })
    handle('saveExport', (_event, raw: unknown) => {
      const request = assertExport(raw)
      const filters = {
        gif: { name: 'Animated GIF', extensions: ['gif'] },
        png: { name: 'PNG image', extensions: ['png'] },
        pgn: { name: 'PGN game', extensions: ['pgn'] },
      }
      return saveWithDialog(window, {
        title: 'Save game',
        defaultPath: join(app.getPath('downloads'), `${request.name}.${request.kind}`),
        filters: [filters[request.kind]],
        data: request.data,
      })
    })
    handle('exportVoiceHistory', () =>
      saveWithDialog(window, {
        title: 'Export voice history',
        defaultPath: `kchess-voice-history-${new Date().toISOString().slice(0, 10)}.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }],
        data: service.voiceHistoryDocument(),
      }),
    )
    handle('notify', (_event, request: unknown) =>
      notify(
        window,
        assertNotification(request),
        (alert) => send(IPC_EVENTS.notification, alert),
        service.settings,
      ),
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
    assertIpcComplete()
    createWindow()
    appUpdates.start()
    app.on('activate', () => {
      if (BrowserWindow.getAllWindows().length === 0) createWindow()
    })
  })
  .catch((error: unknown) => {
    logError('startup', 'Startup failed:', error)
    app.quit()
  })

app.on('window-all-closed', () => {
  core?.suspend()
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
      logWarn('app-updates', 'Could not install on quit:', cause)
    }
  }
})
app.on('will-quit', () => {
  appUpdates?.stop()
  core?.close()
})
