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
import { writeFile } from 'node:fs/promises'
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
import { flushUsage, forgetUsage, resetUsage, usageReport } from './usage'
import { deleteManagedEngine, installManagedEngine } from './managedEngine'
import { withEngineMaintenance } from './uci'
import { analysisRunning, startAnalysis, stopAnalysis } from './analysis'
import {
  cancelReview,
  discardAccountReviews,
  getReview,
  requestReview,
  restartReviewEngine,
  reviewStatus,
  reviewsChanged,
  setupReviews,
  stopReviews,
} from './review'
import { reviewSummaries } from './reviewStore'
import { oauthLook } from './oauthPage'
import {
  OnlineSession,
  cachedProfile,
  connectLichess,
  invalidateLogin,
  cancelAccountSyncs,
  crosstable,
  exportGame,
  playerPerf,
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
  logoutAccounts,
  saveSettings,
} from './store'
import type { ChallengeInfo, OnlineEvent } from '../shared/types'
import { ChallengeInbox } from './challenges'
import { joinTournament, leaveTournament, tournament, tournaments } from './tournaments'
import { Spectator, TV_CHANNEL_KEYS, broadcastTour, broadcasts, tvChannels } from './spectate'
import { cloudEval } from './cloudEval'
import { insights } from './insights'
import { exportToLichessStudy, lichessStudies, lichessStudyChapters } from './studies'
import {
  assertAction,
  assertAnalysisRequest,
  assertChatRoom,
  assertChatText,
  assertDeclineReason,
  assertOptionalAccount,
  assertTournamentId,
  assertLichessId,
  assertPerfType,
  assertExport,
  assertInsightsQuery,
  assertWatchTarget,
  assertTournamentPassword,
  assertTournamentSystem,
  assertActivityMax,
  assertBestMoveOptions,
  assertDays,
  assertFriendList,
  assertGameId,
  assertGameIds,
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
const challengeInbox = new ChallengeInbox((list) => send(IPC_EVENTS.challenges, list))
let ongoingTimer: ReturnType<typeof setTimeout> | undefined
/** Several games start and end together when a stream (re)connects; tell the window once. */
function ongoingChanged(): void {
  clearTimeout(ongoingTimer)
  ongoingTimer = setTimeout(() => send(IPC_EVENTS.ongoingChanged, null), 750)
}
const online = new OnlineSession(
  (event: OnlineEvent) => send(IPC_EVENTS.online, event),
  (message: string) => send(IPC_EVENTS.error, message),
  (state) => {
    if (state.phase === 'checking' || (state.gameId && state.phase !== 'idle')) {
      stopEngine()
      stopAnalysis()
      restartReviewEngine()
      // Your own game needs the connection more than someone else's.
      spectator.stop()
    }
    send(IPC_EVENTS.onlineState, state)
  },
  {
    challenge: (account, event) => {
      const fresh = challengeInbox.ingest(account, event)
      if (!fresh) return
      const control =
        fresh.timeControl.type === 'clock'
          ? `${fresh.timeControl.limit / 60}+${fresh.timeControl.increment}`
          : fresh.timeControl.type === 'correspondence'
            ? `${fresh.timeControl.days} days per move`
            : 'no clock'
      void notify(
        window,
        {
          kind: 'challenge',
          title: fresh.rematchOf
            ? `${fresh.opponent.name} wants a rematch`
            : `${fresh.opponent.name} challenges you`,
          body: `${fresh.variantName} · ${control} · ${fresh.rated ? 'Rated' : 'Casual'} (@${fresh.account})`,
        },
        (alert) => send(IPC_EVENTS.notification, alert),
      ).catch(() => undefined)
    },
    ongoingChanged,
    lobbyState: (state) => send(IPC_EVENTS.lobby, state),
  },
)
const spectator = new Spectator(
  (frame) => send(IPC_EVENTS.watch, frame),
  (update) => send(IPC_EVENTS.broadcast, update),
  (state) => send(IPC_EVENTS.watchState, state),
)
function knownChallenge(id: unknown): ChallengeInfo {
  const challenge = challengeInbox.get(assertGameId(id))
  if (!challenge) throw new Error('That challenge is no longer open.')
  return challenge
}

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
    console.error('Renderer exited:', details.reason, details.exitCode)
  })
  window.webContents.on('preload-error', (_event, _path, error) => {
    console.error('Preload failed:', error)
  })
  window.webContents.on('console-message', ({ level, message }) => {
    if (level === 'warning' || level === 'error') console.error('Renderer:', message)
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
    handle('voiceModelStatus', () => voiceModel.status())
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
    /** Ends everything a signed-in account has running: login, syncs, live play, reviews, usage. */
    async function endSessions(username?: string): Promise<string[]> {
      const accounts = (await loadData()).accounts
        .filter(
          (a) => a.connected && (!username || a.username.toLowerCase() === username.toLowerCase()),
        )
        .map((a) => a.username)
      invalidateLogin(accounts)
      online.logout(username === undefined ? undefined : accounts)
      discardAccountReviews(accounts)
      flushUsage()
      forgetUsage(accounts)
      for (const name of accounts) challengeInbox.forget(name)
      return accounts
    }
    async function logout(username?: string) {
      await endSessions(username)
      const data = await logoutAccounts(username)
      reviewsChanged()
      return data
    }
    handle('logout', (_event, username: unknown) => logout(assertUsername(username)))
    handle('logoutAll', () => logout())
    handle('removeAccount', async (_event, username: unknown) => {
      const name = assertUsername(username)
      // A followed player has no login, but its sync and reviews must still stop before rows go.
      if (!(await endSessions(name)).length) {
        cancelAccountSyncs([name])
        discardAccountReviews([name])
        flushUsage()
        forgetUsage([name])
      }
      challengeInbox.forget(name)
      const data = await removeAccount(name)
      reviewsChanged()
      return data
    })
    handle('syncGames', async (_event, username?: unknown) => {
      const data = await syncGames(username === undefined ? undefined : assertUsername(username))
      reviewsChanged()
      return data
    })
    handle('gamePage', (_event, query: unknown) => gamePage(assertGamePageQuery(query)))
    handle('gameLibraryOverview', () => gameLibraryOverview())
    handle('insights', (_event, query: unknown) => insights(assertInsightsQuery(query)))
    handle('lichessStudies', (_event, account: unknown) => lichessStudies(assertUsername(account)))
    handle('lichessStudyChapters', (_event, account: unknown, id: unknown) =>
      lichessStudyChapters(assertUsername(account), assertLichessId(id)),
    )
    handle(
      'exportToLichessStudy',
      (_event, account: unknown, studyId: unknown, name: unknown, pgn: unknown) => {
        const id = studyId === '' ? '' : assertLichessId(studyId)
        if (typeof name !== 'string' || !name.trim() || name.length > 100)
          throw new Error('Name the chapter (up to 100 characters).')
        if (typeof pgn !== 'string' || !pgn.trim() || pgn.length > 500_000)
          throw new Error('Nothing to export, or the game is too large.')
        return exportToLichessStudy(assertUsername(account), id, name.trim(), pgn)
      },
    )
    handle('gameRatingHistory', (_event, account: unknown) =>
      gameRatingHistory(assertUsername(account)),
    )
    handle('gamePgn', (_event, account: unknown, id: unknown) => {
      return gamePgn(assertUsername(account), assertGameId(id))
    })
    handle('cachedProfile', (_event, username: unknown) => cachedProfile(assertUsername(username)))
    handle('profile', (_event, username: unknown) => profile(assertUsername(username)))
    handle('ratingHistory', (_event, username: unknown) => ratingHistory(assertUsername(username)))
    handle('connectLichess', async (_event, look: unknown) => {
      const bringToFront = (): void => {
        if (!window || window.isDestroyed()) return
        if (window.isMinimized()) window.restore()
        window.show()
        // macOS keeps the browser active unless the app explicitly takes focus.
        app.focus({ steal: true })
        window.focus()
      }
      const connected = await connectLichess(oauthLook(look), bringToFront)
      bringToFront()
      return connected
    })
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
    const replaceEngineFile = (commit: () => Promise<void>): Promise<void> =>
      withEngineMaintenance(commit, () => {
        stopEngine(true)
        stopAnalysis(true)
        restartReviewEngine()
      })
    handle('installEngine', async () => {
      const { path, version, updated } = await installManagedEngine({ replace: replaceEngineFile })
      return { path, version, updated }
    })
    handle('deleteEngine', () => deleteManagedEngine(undefined, replaceEngineFile))
    handle('stopEngine', () => stopEngine())
    handle('bestMove', async (_event, moves: unknown, level: unknown, options?: unknown) => {
      if (online.assistanceBlocked)
        throw new Error('Engine assistance is unavailable during a live Lichess game.')
      return bestMove(
        assertMoves(moves),
        assertLevel(level),
        (await getSettings()).enginePath,
        assertBestMoveOptions(options),
      )
    })
    handle('startAnalysis', async (_event, request: unknown) => {
      if (online.assistanceBlocked)
        throw new Error('Analysis is unavailable during a live Lichess game.')
      return startAnalysis(request, (await getSettings()).enginePath, (update) =>
        send(IPC_EVENTS.analysis, update),
      )
    })
    handle('positionLookup', async (_event, kind: unknown, fen: unknown, options?: unknown) => {
      if (online.assistanceBlocked)
        throw new Error('Position lookups are unavailable during a live Lichess game.')
      return positionLookups.lookup(kind, fen, options)
    })
    handle('exportGame', (_event, id: unknown) => exportGame(assertGameId(id)))
    handle('saveExport', async (_event, raw: unknown) => {
      const request = assertExport(raw)
      const filters = {
        gif: { name: 'Animated GIF', extensions: ['gif'] },
        png: { name: 'PNG image', extensions: ['png'] },
        pgn: { name: 'PGN game', extensions: ['pgn'] },
      }
      const options = {
        title: 'Save game',
        defaultPath: join(app.getPath('downloads'), `${request.name}.${request.kind}`),
        filters: [filters[request.kind]],
      }
      const result =
        window && !window.isDestroyed()
          ? await dialog.showSaveDialog(window, options)
          : await dialog.showSaveDialog(options)
      if (result.canceled || !result.filePath) return false
      await writeFile(result.filePath, request.data)
      return true
    })
    handle('mastersGame', (_event, id: unknown) => {
      if (online.assistanceBlocked)
        throw new Error('Position lookups are unavailable during a live Lichess game.')
      return positionLookups.mastersGame(id)
    })
    handle('cloudEval', async (_event, fen: unknown, lines: unknown) => {
      if (online.assistanceBlocked)
        throw new Error('Analysis is unavailable during a live Lichess game.')
      // Each request sends the position to Lichess, so it needs the setting turned on.
      if (!(await getSettings()).cloudEval)
        throw new Error('Turn on cloud evaluation in Settings → Analysis first.')
      const request = assertAnalysisRequest({ fen, lines })
      return cloudEval(request.fen, request.lines)
    })
    handle('stopAnalysis', () => stopAnalysis())
    handle('reviewGet', (_event, fen: unknown, moves: unknown) => {
      if (online.assistanceBlocked)
        throw new Error('Review is unavailable until your Lichess game status is verified.')
      const request = assertReviewRequest({ fen, moves })
      return getReview(request.fen, request.moves)
    })
    handle('reviewRequest', (_event, request: unknown) => {
      if (online.assistanceBlocked)
        throw new Error('Review is unavailable during a live Lichess game.')
      return requestReview(assertReviewRequest(request))
    })
    handle('reviewCancel', (_event, key: unknown) => cancelReview(assertReviewKey(key)))
    handle('reviewStatus', () => reviewStatus())
    handle('reviewSummaries', (_event, ids: unknown) => reviewSummaries(assertGameIds(ids)))
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
        online.assistanceBlocked
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
    handle('onlineChat', (_event, id: unknown) => online.chat(assertGameId(id)))
    handle('sendChat', (_event, id: unknown, room: unknown, text: unknown) =>
      online.sendChat(assertGameId(id), assertChatRoom(room), assertChatText(text)),
    )
    handle('stayConnected', async (_event, account: unknown) => {
      const name = assertOptionalAccount(account)
      const connected = (await loadData()).accounts.some(
        (entry) => entry.connected && entry.username.toLowerCase() === name.toLowerCase(),
      )
      online.stayConnected(connected ? name : '')
    })
    handle('challenges', () => challengeInbox.list())
    handle('acceptChallenge', async (_event, id: unknown) => {
      const challenge = knownChallenge(id)
      if (challenge.direction !== 'in' || !challenge.playable)
        throw new Error(challenge.problem ?? 'Only challenges sent to you can be accepted.')
      await online.acceptChallenge(challenge)
      challengeInbox.remove(challenge.id)
    })
    handle('declineChallenge', async (_event, id: unknown, reason: unknown) => {
      const challenge = knownChallenge(id)
      if (challenge.direction !== 'in') throw new Error('Withdraw your own challenge instead.')
      await online.declineChallenge(challenge, assertDeclineReason(reason))
      challengeInbox.remove(challenge.id)
    })
    handle('cancelChallenge', async (_event, id: unknown) => {
      const challenge = knownChallenge(id)
      if (challenge.direction !== 'out') throw new Error('Decline a challenge sent to you instead.')
      await online.withdrawChallenge(challenge)
      challengeInbox.remove(challenge.id)
    })
    handle('ongoingGames', () => online.ongoing())
    handle('tournaments', (_event, account: unknown) => tournaments(assertOptionalAccount(account)))
    handle('tvChannels', () => tvChannels())
    handle('playerPerf', (_event, username: unknown, perf: unknown) =>
      playerPerf(assertUsername(username), assertPerfType(perf)),
    )
    handle('crosstable', (_event, a: unknown, b: unknown) =>
      crosstable(assertUsername(a), assertUsername(b)),
    )
    handle('watch', (_event, target: unknown) => {
      if (online.playing) throw new Error('Finish your game before watching another.')
      return spectator.watch(assertWatchTarget(target, TV_CHANNEL_KEYS))
    })
    handle('watchBroadcast', (_event, roundId: unknown) => {
      if (online.playing) throw new Error('Finish your game before watching another.')
      return spectator.watchRound(assertLichessId(roundId))
    })
    handle('stopWatching', () => spectator.stop())
    handle('broadcasts', () => broadcasts())
    handle('broadcastTour', (_event, id: unknown) => broadcastTour(assertLichessId(id)))
    handle('tournament', (_event, system: unknown, id: unknown, account: unknown) =>
      tournament(
        assertTournamentSystem(system),
        assertTournamentId(id),
        assertOptionalAccount(account),
      ),
    )
    handle(
      'joinTournament',
      async (_event, system: unknown, id: unknown, account: unknown, password: unknown) => {
        const name = assertUsername(account)
        const result = await joinTournament(
          assertTournamentSystem(system),
          assertTournamentId(id),
          name,
          assertTournamentPassword(password),
        )
        // Pairings arrive on the event stream: keep it open while in the tournament.
        if (result === true) online.stayConnected(name)
        return result
      },
    )
    handle('leaveTournament', (_event, system: unknown, id: unknown, account: unknown) =>
      leaveTournament(
        assertTournamentSystem(system),
        assertTournamentId(id),
        assertUsername(account),
      ),
    )
    handle('openGame', (_event, account: unknown, id: unknown) =>
      online.open(assertUsername(account), assertGameId(id)),
    )
    handle('clearAccountData', async (_event, username: unknown) => {
      const name = assertUsername(username)
      // Stop a running sync and review first, or they would write back into the cleared library.
      cancelAccountSyncs([name])
      discardAccountReviews([name])
      const data = await clearAccountData(name)
      forgetProfile(name)
      reviewsChanged()
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
  spectator.stop()
  // Nothing can answer a challenge without a window; a new window asks to stay connected again.
  online.close()
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
  online.close()
  stopEngine(true)
  stopAnalysis(true)
  stopReviews()
  flushUsage()
  closePuzzleWorker()
  closeDb()
})
