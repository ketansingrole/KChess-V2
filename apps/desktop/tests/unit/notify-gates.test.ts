import { expect, it, vi } from 'vitest'
import { DEFAULT_SETTINGS } from '@kchess/contracts/defaultSettings'

vi.mock('electron', () => ({ app: {}, Notification: { isSupported: () => false } }))

const { notify } = await import('../../electron/main/notify')

const focused = {
  isDestroyed: () => false,
  isFocused: () => true,
  isVisible: () => true,
  isMinimized: () => false,
}
const test = { kind: 'test' as const, title: 'KChess notifications', body: 'Sample' }

// "Send test" showed a notification with Desktop notifications switched off.
it('keeps the test notification behind the master switch', async () => {
  const alert = vi.fn()
  const off = async () => ({ ...DEFAULT_SETTINGS, notificationsEnabled: false })
  expect(await notify(focused as never, test, alert, off)).toEqual({
    shown: false,
    skipped: 'disabled',
  })
  expect(alert).not.toHaveBeenCalled()
  const on = async () => ({ ...DEFAULT_SETTINGS, notificationsEnabled: true, notifyActive: false })
  expect(await notify(focused as never, test, alert, on)).toEqual({ shown: true, via: 'in-app' })
})

// Two KChess windows on one profile (docs/shared-core.md): only the one focused last alerts.
it('stays quiet while another KChess window raises the alerts', async () => {
  const alert = vi.fn()
  const on = async () => ({ ...DEFAULT_SETTINGS, notificationsEnabled: true, notifyActive: true })
  const move = { kind: 'opponentMove' as const, title: 'Your move', body: 'e5' }
  expect(await notify(focused as never, move, alert, on, () => false)).toEqual({
    shown: false,
    skipped: 'window-state',
  })
  expect(alert).not.toHaveBeenCalled()
  expect(await notify(focused as never, move, alert, on, () => true)).toEqual({
    shown: true,
    via: 'in-app',
  })
  // The test notification is the person's own request: it shows in whichever window asked.
  expect(await notify(focused as never, test, alert, on, () => false)).toMatchObject({
    shown: true,
  })
})
