/** Matches `build.appId` and `build.productName` in apps/desktop/package.json (checked by a unit test). */
export const APP_ID = 'com.kchess.app'
export const APP_NAME = 'KChess'

/**
 * The Windows AppUserModelID for this process. electron-builder stamps APP_ID on the installed
 * Start-menu shortcut, so an installed app must use it for notifications and taskbar grouping to
 * match the shortcut; otherwise Windows shows Electron's fallback ID. Unpackaged runs have no such
 * shortcut, and use the executable path (Electron's suggestion for development) so they do not
 * share a taskbar group or notification history with an installed copy.
 */
export function appUserModelId(installed: boolean, execPath: string): string {
  return installed ? APP_ID : execPath
}
