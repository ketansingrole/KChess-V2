import { app } from 'electron'
import updaterPackage from 'electron-updater'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { AppUpdates } from './appUpdates'
import { getSettings } from './store'
import type { AppUpdateStatus } from '../shared/types'

export const APP_RELEASES_URL = 'https://github.com/ketansingrole/KChess-V2/releases'

/** Configures the public stable feed and capabilities of this installed package. */
export async function setupAppUpdates(
  emit: (status: AppUpdateStatus) => void,
  onInstallFailure: () => void,
): Promise<AppUpdates> {
  const { autoUpdater } = updaterPackage
  autoUpdater.logger = console
  const packaged = app.isPackaged && existsSync(join(process.resourcesPath, 'app-update.yml'))
  let supported = process.platform === 'darwin' || process.platform === 'win32'
  if (process.platform === 'linux') {
    let packageType = ''
    try {
      packageType = readFileSync(join(process.resourcesPath, 'package-type'), 'utf8').trim()
    } catch {
      // An unpacked directory has no package type. It cannot replace itself.
    }
    supported = Boolean(process.env.APPIMAGE) || packageType === 'deb'
  }
  let macSigned = false
  if (packaged && process.platform === 'darwin') {
    const metadata = JSON.parse(readFileSync(join(app.getAppPath(), 'package.json'), 'utf8')) as {
      kchessMacAutoUpdate?: boolean
    }
    macSigned = metadata.kchessMacAutoUpdate === true
  }
  const canCheck = packaged && supported
  const canInstall = canCheck && (process.platform !== 'darwin' || macSigned)
  const reason = !app.isPackaged
    ? 'Updates are available in installed release builds. Development builds do not check or install updates.'
    : !canCheck
      ? 'This installation cannot update itself. Download and install the latest release.'
      : !canInstall
        ? 'This Mac build can check for updates. Download and install new releases manually until you install a Developer ID signed version.'
        : undefined
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
