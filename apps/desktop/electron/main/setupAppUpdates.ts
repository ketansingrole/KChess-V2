import { app } from 'electron'
import updaterPackage from 'electron-updater'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { AppUpdates } from './appUpdates'
import { logDebug, logError, logWarn } from '@kchess/native/logger'
import type { AppUpdateStatus, Settings } from '@kchess/contracts/types'

export const APP_RELEASES_URL = 'https://github.com/ketansingrole/KChess-V2/releases'

export interface UpdateHost {
  /** True in `electron .` / dev (matches the old `!app.isPackaged` reason branch). */
  dev: boolean
  /** True when the installed package ships an update feed (`app-update.yml`). */
  hasFeed: boolean
  platform: NodeJS.Platform
  /** Contents of `<resources>/package-type` on Linux (`deb`, `AppImage`, …); undefined elsewhere. */
  packageType?: string
  appImage?: string
  macSigned: boolean
}

/** Pure capability matrix, tested without Electron: which packages may check and self-install. */
export function resolveUpdateCapabilities(host: UpdateHost): {
  canCheck: boolean
  canInstall: boolean
  reason?: string
} {
  let supported = host.platform === 'darwin' || host.platform === 'win32'
  if (host.platform === 'linux') supported = Boolean(host.appImage) || host.packageType === 'deb'
  const canCheck = !host.dev && host.hasFeed && supported
  const canInstall = canCheck && (host.platform !== 'darwin' || host.macSigned)
  const reason = host.dev
    ? 'Updates are available in installed release builds. Development builds do not check or install updates.'
    : !canCheck
      ? 'This installation cannot update itself. Download and install the latest release.'
      : !canInstall
        ? 'This Mac build can check for updates. Download and install new releases manually until you install a Developer ID signed version.'
        : undefined
  return { canCheck, canInstall, reason }
}

/** Configures the public stable feed and capabilities of this installed package. */
export async function setupAppUpdates(
  emit: (status: AppUpdateStatus) => void,
  onInstallFailure: () => void,
  getSettings: () => Promise<Settings>,
): Promise<AppUpdates> {
  const { autoUpdater } = updaterPackage
  autoUpdater.logger = {
    info: (message?: unknown) => logDebug('app-updates', String(message)),
    warn: (message?: unknown) => logWarn('app-updates', String(message)),
    error: (message?: unknown) => logError('app-updates', String(message)),
    debug: (message?: string) => logDebug('app-updates', String(message)),
  }
  const packaged = app.isPackaged && existsSync(join(process.resourcesPath, 'app-update.yml'))
  let packageType = ''
  if (process.platform === 'linux') {
    try {
      packageType = readFileSync(join(process.resourcesPath, 'package-type'), 'utf8').trim()
    } catch (cause) {
      logDebug('app-updates', 'No Linux package type; self-update is unavailable:', cause)
    }
  }
  let macSigned = false
  if (packaged && process.platform === 'darwin') {
    const metadata = JSON.parse(readFileSync(join(app.getAppPath(), 'package.json'), 'utf8')) as {
      kchessMacAutoUpdate?: boolean
    }
    macSigned = metadata.kchessMacAutoUpdate === true
  }
  const { canCheck, canInstall, reason } = resolveUpdateCapabilities({
    dev: !app.isPackaged,
    hasFeed: packaged,
    platform: process.platform,
    packageType,
    appImage: process.env.APPIMAGE,
    macSigned,
  })
  const updates = new AppUpdates(
    autoUpdater,
    { currentVersion: app.getVersion(), canCheck, canInstall, reason },
    getSettings,
    emit,
    onInstallFailure,
  )
  updates.applyPreferences(await getSettings())
  return updates
}
