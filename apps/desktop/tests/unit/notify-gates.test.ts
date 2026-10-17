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
