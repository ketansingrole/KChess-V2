import type { BrowserWindow } from 'electron'
import { app, Notification } from 'electron'
import { logError } from '../../../../core/src/services/logger'
import type {
  NotificationKind,
  NotificationRequest,
  NotificationResult,
  Settings,
} from '../../../../core/src/contracts/types'

/** The Settings switch that governs each kind; a test notification is always allowed. */
const CATEGORY: Record<Exclude<NotificationKind, 'test'>, keyof Settings> = {
  opponentMove: 'notifyOpponentMove',
  lowTime: 'notifyLowTime',
  gameEvents: 'notifyGameEvents',
  computerMove: 'notifyComputerMove',
  challenge: 'notifyChallenges',
}

/** Electron drops a notification (and its click handler) if it is garbage collected while shown. */
const live = new Set<Notification>()

/** KChess is in the background: unfocused, minimized, hidden or closed (macOS keeps running without a window). */
function isAway(window: BrowserWindow | null): boolean {
  return (
    !window ||
    window.isDestroyed() ||
    !window.isFocused() ||
    window.isMinimized() ||
    !window.isVisible()
  )
}

function reveal(window: BrowserWindow | null): void {
  if (!window || window.isDestroyed()) return
  if (window.isMinimized()) window.restore()
  window.show()
  window.focus()
  if (process.platform === 'darwin') app.focus({ steal: true })
}

/**
 * Alert the player when Settings allow it. The rules live here, not in the renderer, so a
 * buggy page cannot bypass them and the window state is read first-hand.
 *
 * While KChess is in use the alert is shown inside the window (`alert`): macOS does not show
 * banners for the app in front, so a system notification would silently vanish. In the
 * background it is a real system notification.
 */
export async function notify(
  window: BrowserWindow | null,
  request: NotificationRequest,
  alert: (payload: { title: string; body: string }) => void,
  readSettings: () => Promise<Settings>,
): Promise<NotificationResult> {
  const settings = await readSettings()
  const away = isAway(window)
  if (request.kind !== 'test') {
    if (!settings.notificationsEnabled) return { shown: false, skipped: 'disabled' }
    if (!settings[CATEGORY[request.kind]]) return { shown: false, skipped: 'category-off' }
    if (!(away ? settings.notifyBackground : settings.notifyActive))
      return { shown: false, skipped: 'window-state' }
  }
  if (!away) {
    alert({ title: request.title, body: request.body })
    return { shown: true, via: 'in-app' }
  }
  if (!Notification.isSupported()) return { shown: false, skipped: 'unsupported' }
  const notification = new Notification({
    title: request.title,
    body: request.body,
    silent: !settings.notifySound,
  })
  live.add(notification)
  const forget = (): void => void live.delete(notification)
  notification.on('click', () => {
    reveal(window)
    forget()
  })
  notification.on('close', forget)
  // The system reports back asynchronously: `show` when it accepted the notification, `failed`
  // (e.g. permission denied) otherwise. Wait briefly so the caller can tell the two apart.
  const outcome = new Promise<NotificationResult>((resolve) => {
    notification.once('show', () => resolve({ shown: true, via: 'system' }))
    notification.once('failed', (_event, error) => {
      forget()
      logError('notify', 'Notification failed:', error)
      resolve({ shown: false, skipped: 'failed', error })
    })
    setTimeout(() => resolve({ shown: true, via: 'system' }), 1500)
  })
  notification.show()
  return outcome
}
