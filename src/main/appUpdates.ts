import type { AppUpdater } from 'electron-updater'
import { logDebug, logInfo, logWarn } from './logger'
import type { AppUpdateStatus, Settings } from '../shared/types'

export type UpdatePreferences = Pick<
  Settings,
  'updateAutoCheck' | 'updateAutoDownload' | 'updateInstallOnQuit'
>
type Updater = Pick<
  AppUpdater,
  | 'on'
  | 'autoDownload'
  | 'autoInstallOnAppQuit'
  | 'autoRunAppAfterInstall'
  | 'allowPrerelease'
  | 'allowDowngrade'
  | 'disableWebInstaller'
  | 'checkForUpdates'
  | 'downloadUpdate'
  | 'quitAndInstall'
>

/** Main-process owner of update state. The renderer never receives download URLs or file paths. */
export class AppUpdates {
  private state: AppUpdateStatus
  private checking?: Promise<AppUpdateStatus>
  private downloading?: Promise<AppUpdateStatus>
  private startup?: ReturnType<typeof setTimeout>
  private interval?: ReturnType<typeof setInterval>
  private stopped = false
  private installWhenQuitting = false
  private installing = false

  constructor(
    private updater: Updater,
    capabilities: Pick<AppUpdateStatus, 'currentVersion' | 'canCheck' | 'canInstall' | 'reason'>,
    private preferences: () => Promise<UpdatePreferences>,
    private emit: (status: AppUpdateStatus) => void,
    private onInstallFailure?: () => void,
  ) {
    this.state = { ...capabilities, phase: capabilities.canCheck ? 'idle' : 'disabled' }
    // Own downloads ourselves so preferences changed during a check are honoured.
    updater.autoDownload = false
    updater.autoInstallOnAppQuit = false
    updater.autoRunAppAfterInstall = false
    updater.allowPrerelease = false
    updater.allowDowngrade = false
    updater.disableWebInstaller = true
    updater.on('update-available', (info) => {
      logInfo('app-updates', 'Update available:', `version=${info.version ?? 'unknown'}`)
      this.set({ phase: 'available', version: info.version, releaseDate: info.releaseDate })
    })
    updater.on('update-not-available', () => {
      logInfo('app-updates', 'Update check: up-to-date', `current=${this.state.currentVersion}`)
      this.set({ phase: 'up-to-date', version: undefined, releaseDate: undefined })
    })
    updater.on('download-progress', (progress) => {
      this.set({ phase: 'downloading', progress })
    })
    updater.on('update-downloaded', (info) => {
      logInfo('app-updates', 'Update downloaded:', `version=${info.version ?? 'unknown'}`)
      this.set({
        phase: 'downloaded',
        version: info.version,
        progress: undefined,
        error: undefined,
      })
    })
    // An error listener is also required by EventEmitter: a network failure must never crash KChess.
    updater.on('error', (error) => this.failed(error))
  }

  /** Returns a defensive snapshot for IPC and callers. */
  status(): AppUpdateStatus {
    return structuredClone(this.state)
  }

  private set(patch: Partial<AppUpdateStatus>): void {
    this.state = { ...this.state, ...patch }
    if (!this.stopped) this.emit(this.status())
  }

  private failed(cause: unknown): void {
    const wasInstalling = this.installing
    this.installing = false
    logWarn(
      'app-updates',
      'App update failed:',
      `current=${this.state.currentVersion}`,
      this.state.version ? `available=${this.state.version}` : 'available=none',
      `phase=${this.state.phase}`,
      cause,
    )
    // A preference read or an unrelated late error does not invalidate the verified installer.
    if (this.state.phase === 'downloaded' && !wasInstalling) return
    this.set({
      phase: 'error',
      progress: undefined,
      error:
        'Could not update KChess. Check your internet connection and try again. If it keeps failing, download the latest release or export diagnostics.',
    })
    if (wasInstalling) this.onInstallFailure?.()
  }

  /** Applies quit behaviour immediately, including after an installer has downloaded. */
  applyPreferences(settings: UpdatePreferences): void {
    // Own quit installation as well: the library only registers its quit handler at download
    // time, so changing this preference after downloading would otherwise be ignored.
    this.installWhenQuitting = settings.updateInstallOnQuit
  }

  /** Checks are shared; checking again must not discard a verified download ready to install. */
  check(): Promise<AppUpdateStatus> {
    if (!this.state.canCheck || this.stopped) return Promise.resolve(this.status())
    if (this.checking) return this.checking
    if (this.downloading || this.state.phase === 'downloaded') return Promise.resolve(this.status())
    this.checking = this.performCheck().finally(() => {
      this.checking = undefined
    })
    return this.checking
  }

  private async performCheck(): Promise<AppUpdateStatus> {
    this.set({ phase: 'checking', error: undefined, version: undefined, progress: undefined })
    logDebug('app-updates', 'Checking for updates:', `current=${this.state.currentVersion}`)
    try {
      const result = await this.updater.checkForUpdates()
      if (!result) throw new Error('This installation cannot check for updates.')
      this.set({ checkedAt: Date.now() })
      const settings = await this.preferences()
      this.applyPreferences(settings)
      if (this.state.phase === 'available' && this.state.canInstall && settings.updateAutoDownload)
        void this.download()
    } catch (cause) {
      logDebug('app-updates', 'Update check failed:', cause)
      this.failed(cause)
    }
    return this.status()
  }

  /** Shares an active download and permits retrying failed downloads or installations. */
  download(): Promise<AppUpdateStatus> {
    if (this.downloading) return this.downloading
    if (
      this.stopped ||
      !this.state.canInstall ||
      !this.state.version ||
      !['available', 'error'].includes(this.state.phase)
    )
      return Promise.resolve(this.status())
    this.downloading = this.performDownload().finally(() => {
      this.downloading = undefined
    })
    return this.downloading
  }

  private async performDownload(): Promise<AppUpdateStatus> {
    this.set({ phase: 'downloading', error: undefined, progress: undefined })
    try {
      this.applyPreferences(await this.preferences())
      await this.updater.downloadUpdate()
    } catch (cause) {
      logDebug('app-updates', 'Update download failed:', cause)
      this.failed(cause)
    }
    return this.status()
  }

  /** Reports whether the next user quit should apply a verified installer. */
  shouldInstallOnQuit(): boolean {
    return (
      this.installWhenQuitting &&
      !this.installing &&
      this.state.canInstall &&
      this.state.phase === 'downloaded'
    )
  }

  /** Applies a verified installer; normal quits must not relaunch the app. */
  install(restart = true): void {
    if (!this.state.canInstall || this.state.phase !== 'downloaded')
      throw new Error('Download an update before restarting KChess.')
    if (this.installing) return
    this.installing = true
    // A normal quit must stay quit; only this explicit action requests a relaunch.
    this.updater.autoRunAppAfterInstall = restart
    try {
      this.updater.quitAndInstall(!restart, restart)
    } catch (cause) {
      this.failed(cause)
      throw cause
    }
  }

  /** Schedules preference-aware background checks without delaying app startup. */
  start(): void {
    if (!this.state.canCheck || this.startup || this.interval) return
    // Let startup, engine loading and the online-game reconnection finish first.
    this.startup = setTimeout(() => void this.checkAutomatically(), 10_000)
    this.interval = setInterval(() => void this.checkAutomatically(), 6 * 60 * 60 * 1000)
    this.startup.unref?.()
    this.interval.unref?.()
  }

  private async checkAutomatically(): Promise<void> {
    try {
      if (!this.stopped && (await this.preferences()).updateAutoCheck) await this.check()
    } catch (cause) {
      logDebug('app-updates', 'Automatic update check failed:', cause)
      this.failed(cause)
    }
  }

  /** Cancels background checks and stops forwarding update events at shutdown. */
  stop(): void {
    this.stopped = true
    clearTimeout(this.startup)
    clearInterval(this.interval)
  }
}
