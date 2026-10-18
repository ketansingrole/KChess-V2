import { QuitPreparation, type QuitRetryChoice } from './quit'
import { PERFORMANCE_NAMES, type PerformanceName } from '../../contracts/rendererDiagnostics'
import {
  app,
  BrowserWindow,
  dialog,
  Menu,
  nativeImage,
  nativeTheme,
  screen,
  shell,
  type MenuItemConstructorOptions,
} from 'electron'
import { handleAppProtocol, registerAppScheme } from './appProtocol'
import { APP_NAME, appUserModelId } from './appIdentity'
import { microphoneAccess, openMicrophoneSettings, setupMediaPermissions } from './microphone'
import { handle, setIpcOwner, assertIpcComplete } from './ipc'
import { isAppUrl } from './appOrigin'
import {
  FORWARDED_CORE_EVENTS,
  IPC_EVENTS,
  type IpcArguments,
  type IpcEvents,
  type IpcResult,
} from '../../contracts/ipc'
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
  VoiceModelCache,
  type CoreMethod,
  type KChessCore,
} from '@kchess/native'
import { electronPlatform } from './platform'
import { saveWithDialog } from './saveDialog'
import {
  applyWindowMode,
  MIN_SIZE,
  readWindowState,
  restoredBounds,
  trackWindowState,
} from './windowState'
import { assertExport, assertNotification } from '@kchess/rules/validate'
import { InterfaceZoomController } from './interfaceZoom'
import { isInterfaceZoom } from '../../contracts/interfaceZoom'
import type { ChallengeInfo } from '../../contracts/types'

let window: BrowserWindow | null = null
let appUpdates: AppUpdates | undefined
let interfaceZoom: InterfaceZoomController | undefined
let core: KChessCore | undefined
/** Set when the renderer process exits; cleared on a new or reloaded window. */
let rendererGone = false
const quitPreparation = new QuitPreparation({
  window: () => {
    const owner = window
    if (!owner || owner.isDestroyed()) return undefined
    return {
      send: (token) => owner.webContents.send(IPC_EVENTS.prepareQuit, token),
      onClosed(callback) {
        owner.once('closed', callback)
        return () => owner.removeListener('closed', callback)
      },
      gone: () => rendererGone,
    }
  },
  async retry(): Promise<QuitRetryChoice> {
    const options = {
      type: 'warning' as const,
      title: 'Could not save before quitting',
      message: 'Some changes could not be saved.',
      detail: 'Retry saving, quit without saving your changes, or cancel to keep them open.',
      buttons: ['Retry', 'Quit without saving', 'Cancel'],
      defaultId: 2,
      cancelId: 2,
      noLink: true,
    }
    const owner = window
    const result =
      owner && !owner.isDestroyed()
        ? await dialog.showMessageBox(owner, options)
        : await dialog.showMessageBox(options)
    return result.response === 0 ? 'retry' : result.response === 1 ? 'discard' : 'cancel'
  },
})
setIpcOwner(() => window)
registerAppScheme()
app.setName(APP_NAME)
if (process.platform === 'win32') {
  // The branded dev binary reports isPackaged, so also check how it was started.
  const installed = app.isPackaged && process.defaultApp !== true && !process.env.KCHESS_NUXT_URL
  app.setAppUserModelId(appUserModelId(installed, process.execPath))
}
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
    service.desktopSettings,
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
    handle(method, async (_event, ...args) => {
      if (method === 'saveSettings')
        return (await service.saveDesktopSettings(
          args[0] as import('../../contracts/types').Settings,
        )) as never
      const result = await call(...(args as IpcArguments<CoreMethod>))
      if (result && typeof result === 'object' && 'settings' in result && 'accounts' in result)
        return { ...result, settings: await service.desktopSettings() } as never
      if (method === 'connectLichess' && result && typeof result === 'object' && 'data' in result)
        return {
          ...result,
          data: { ...(result.data as object), settings: await service.desktopSettings() },
        } as never
      return result as never
    })
  }
  for (const event of FORWARDED_CORE_EVENTS)
    service.on(event, (payload) => send(event, payload as never))
  service.on('challenge:received', (challenge) => challengeAlert(service, challenge))
  service.on('settings:saved', () => {
    void service
      .desktopSettings()
      .then((settings) => appUpdates?.applyPreferences(settings))
      .catch((cause) => logWarn('settings', 'Could not apply desktop preferences:', cause))
  })
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
  rendererGone = false
  const icon = windowIcon()
  // macOS keeps its traffic lights (hiddenInset). Windows/Linux use OpenChamber-style
  // frameless chrome: no OS title bar at all, the renderer draws its own
  // minimize/maximize/close buttons in the topbar. The menu bar stays available
  // via Alt but is hidden so the window doesn't look like a legacy Win32 app.
  const frameless = process.platform === 'win32' || process.platform === 'linux'
  // Reopen where and how the window was left, if a connected display can still show it.
  const stateFile = join(app.getPath('userData'), 'window-state.json')
  const saved = readWindowState(stateFile)
  window = new BrowserWindow({
    title: 'KChess',
    ...restoredBounds(
      saved,
      screen.getAllDisplays().map((display) => display.workArea),
      screen.getPrimaryDisplay().workArea,
    ),
    minWidth: MIN_SIZE.width,
    minHeight: MIN_SIZE.height,
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
  applyWindowMode(window, saved)
  trackWindowState(window, stateFile)
  interfaceZoom?.attach(window)
  window.on('closed', () => {
    window = null
  })
  const owner = window
  let closingWindow = false
  let preparingClose = false
  owner.on('close', (event) => {
    if (coreStopped || closingWindow) return
    event.preventDefault()
    if (coreShutdown || preparingClose) return
    preparingClose = true
    void quitPreparation
      .prepare()
      .then((saved) => {
        preparingClose = false
        if (!saved || owner.isDestroyed() || coreShutdown) return
        closingWindow = true
        owner.close()
      })
      .catch((cause) => {
        preparingClose = false
        logWarn('shutdown', 'Window close preparation failed:', cause)
      })
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
    // A crashed renderer holds no flushable state; quitting must not wait on it.
    rendererGone = true
    logError('renderer', 'Renderer exited:', details.reason, details.exitCode)
  })
  window.webContents.on('preload-error', (_event, _path, error) => {
    logError('renderer', 'Preload failed:', error)
  })
  window.webContents.on('console-message', ({ level, message }) => {
    if (level === 'warning') logWarn('renderer', 'Renderer console:', message)
    else if (level === 'error') logError('renderer', 'Renderer console:', message)
  })
  window.webContents.on('did-fail-load', (_event, code, description, url) =>
    logError('renderer', 'Renderer load failed:', code, description, url),
  )
  // A reloaded renderer can flush again.
  window.webContents.on('did-finish-load', () => {
    rendererGone = false
  })
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
  // View keeps the standard items, but zoom goes through the interface zoom so it is saved,
  // shown in Settings and reached by Ctrl/Cmd with "=" as well as "+". The accelerators are
  // labels: the window handles the keys itself (see InterfaceZoomController).
  const zoom = (direction: 1 | -1 | 0) => () => interfaceZoom?.step(direction)
  const view: MenuItemConstructorOptions = {
    label: 'View',
    submenu: [
      { role: 'reload' },
      { role: 'forceReload' },
      { role: 'toggleDevTools' },
      { type: 'separator' },
      { label: 'Actual Size', accelerator: 'CmdOrCtrl+0', click: zoom(0) },
      { label: 'Zoom In', accelerator: 'CmdOrCtrl+=', click: zoom(1) },
      { label: 'Zoom Out', accelerator: 'CmdOrCtrl+-', click: zoom(-1) },
      { type: 'separator' },
      { role: 'togglefullscreen' },
    ],
  }
  // macOS: label the application menu with the app name (About KChess,
  // Hide KChess, Quit KChess, …) instead of leaving Electron defaults.
  const appMenu: MenuItemConstructorOptions[] =
    process.platform === 'darwin'
      ? [
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
        ]
      : []
  Menu.setApplicationMenu(
    Menu.buildFromTemplate([
      ...appMenu,
      { role: 'fileMenu' },
      { role: 'editMenu' },
      view,
      { role: 'windowMenu' },
    ]),
  )
}

// One app instance per profile: a second launch (common on Windows/Linux) quits
// here while the running instance restores its window. Without this, two cores
// would drive the same databases, engine scheduler and Lichess streams.
const singleInstance = app.requestSingleInstanceLock()
if (!singleInstance) {
  app.quit()
} else {
  app.on('second-instance', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      if (!coreShutdown) createWindow()
    } else bringToFront()
  })
}

void app
  .whenReady()
  .then(async () => {
    // A second instance already quit above; its ready handler must not start a core.
    if (!singleInstance) return
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
    const zoom = new InterfaceZoomController(join(app.getPath('userData'), 'interface-zoom.json'), {
      workAreaOf: (bounds) => screen.getDisplayMatching(bounds).workArea,
      onDisplaysChanged(listener) {
        const events = ['display-added', 'display-removed', 'display-metrics-changed'] as const
        for (const event of events) screen.on(event as 'display-added', listener)
        return () => {
          for (const event of events) screen.removeListener(event as 'display-added', listener)
        }
      },
    })
    interfaceZoom = zoom
    zoom.onChange((state) => send(IPC_EVENTS.interfaceZoom, state))
    handle('interfaceZoom', () => zoom.state())
    handle('setInterfaceZoom', (_event, setting: unknown) => {
      if (!isInterfaceZoom(setting)) throw new Error('Invalid interface zoom.')
      return zoom.set(setting)
    })
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
      service.desktopSettings,
    )
    const updates = appUpdates
    if (!updates) throw new Error('App updates are unavailable.')
    handle('appUpdateStatus', () => updates.status())
    handle('checkAppUpdate', () => updates.check())
    handle('downloadAppUpdate', () => updates.download())
    handle('installAppUpdate', async () => {
      if (await quitPreparation.prepare()) updates.install()
    })
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
        service.desktopSettings,
      ),
    )
    handle('loadThemes', () => loadCustomThemes())
    handle('openThemesFolder', async () => {
      await loadCustomThemes() // makes sure the folder exists
      // openPath reports failure as a message rather than a rejection.
      const problem = await shell.openPath(themesDir())
      if (problem) throw new Error(`Could not open the themes folder: ${problem}`)
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
    handle('completeQuit', (_event, token, saved) => {
      if (typeof token !== 'string' || (saved !== undefined && typeof saved !== 'boolean'))
        throw new Error('Invalid quit acknowledgment.')
      quitPreparation.complete(token, saved)
    })
    assertIpcComplete()
    createWindow()
    appUpdates.start()
    app.on('activate', () => {
      if (!coreShutdown && BrowserWindow.getAllWindows().length === 0) createWindow()
    })
  })
  .catch((error: unknown) => {
    logError('startup', 'Startup failed:', error)
    app.quit()
  })

app.on('window-all-closed', () => {
  if (!coreShutdown) core?.suspend()
  if (process.platform !== 'darwin') app.quit()
})
let coreShutdown: Promise<void> | undefined
let coreStopped = false
let installPreparation: Promise<void> | undefined
app.on('before-quit', (event) => {
  if (appUpdates?.shouldInstallOnQuit()) {
    event.preventDefault()
    installPreparation ??= (async () => {
      if (await quitPreparation.prepare()) appUpdates?.install(false)
    })()
      .catch((cause) => {
        logWarn('app-updates', 'Could not install on quit:', cause)
      })
      .finally(() => {
        installPreparation = undefined
      })
    return
  }
  if (core && !coreStopped) {
    event.preventDefault()
    coreShutdown ??= (async () => {
      // An update install already flushed the renderer before calling quitAndInstall;
      // re-preparing here would only add a second 3s-timeout and retry dialog.
      if (!appUpdates?.isInstalling() && !(await quitPreparation.prepare())) return
      await core?.close()
      coreStopped = true
      app.quit()
    })()
      .catch((cause) => {
        logError('shutdown', 'Core shutdown failed:', cause)
        coreStopped = true
        app.quit()
      })
      .finally(() => {
        if (!coreStopped) coreShutdown = undefined
      })
  }
})
app.on('will-quit', () => {
  appUpdates?.stop()
})
