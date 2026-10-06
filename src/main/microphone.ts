import { app, session, shell, systemPreferences } from 'electron'
import { logDebug } from './logger'
import type { MicrophoneAccess, MicrophoneStatus } from '../shared/types'

function status(): MicrophoneStatus {
  if (process.platform !== 'darwin' && process.platform !== 'win32') return 'granted'
  try {
    const value = systemPreferences.getMediaAccessStatus('microphone')
    return value === 'granted' ||
      value === 'denied' ||
      value === 'restricted' ||
      value === 'not-determined'
      ? value
      : 'unknown'
  } catch (cause) {
    logDebug('microphone', 'Could not read microphone access status:', cause)
    return 'unknown'
  }
}

function report(value: MicrophoneStatus): MicrophoneAccess {
  return {
    status: value,
    canOpenSettings: process.platform === 'darwin' || process.platform === 'win32',
    // macOS attributes a dev build's mic use to the app that launched it (a terminal or editor).
    // The branded dev binary reports isPackaged, so also check how it was started.
    launchedFromTerminal:
      process.platform === 'darwin' &&
      (!app.isPackaged || process.defaultApp === true || !!process.env.KCHESS_NUXT_URL),
  }
}

/**
 * Electron never asks macOS for microphone access on its own: getUserMedia then fails with
 * NotReadableError while the status is "not-determined". Ask explicitly so the system prompt shows.
 */
export async function microphoneAccess(request: boolean): Promise<MicrophoneAccess> {
  const current = status()
  if (!request || current !== 'not-determined' || process.platform !== 'darwin')
    return report(current)
  try {
    await systemPreferences.askForMediaAccess('microphone')
  } catch (cause) {
    logDebug('microphone', 'Microphone access request failed:', cause)
  }
  return report(status())
}

export async function openMicrophoneSettings(): Promise<boolean> {
  // Fixed URLs only: nothing from the renderer reaches openExternal.
  const url =
    process.platform === 'darwin'
      ? 'x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone'
      : process.platform === 'win32'
        ? 'ms-settings:privacy-microphone'
        : undefined
  if (!url) return false
  await shell.openExternal(url)
  return true
}

/** Grant audio capture to the app's own pages only; never camera or screen capture. */
export function setupMediaPermissions(isAppUrl: (url: string) => boolean): void {
  const ses = session.defaultSession
  ses.setPermissionCheckHandler((contents, permission, origin, details) =>
    Boolean(
      contents &&
      permission === 'media' &&
      isAppUrl(contents.getURL()) &&
      isAppUrl(origin) &&
      details.mediaType === 'audio',
    ),
  )
  ses.setPermissionRequestHandler((contents, permission, callback, details) => {
    if (permission !== 'media') return callback(false)
    const types = 'mediaTypes' in details ? (details.mediaTypes ?? []) : []
    callback(
      isAppUrl(contents.getURL()) &&
        isAppUrl(details.requestingUrl) &&
        details.isMainFrame &&
        types.length > 0 &&
        types.every((t) => t === 'audio'),
    )
  })
}
