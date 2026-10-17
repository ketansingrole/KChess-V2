import { expect, it, vi } from 'vitest'

const handlers = vi.hoisted(() => ({
  check: undefined as undefined | ((...args: unknown[]) => boolean),
  request: undefined as undefined | ((...args: unknown[]) => void),
}))

vi.mock('electron', () => ({
  app: {},
  shell: {},
  systemPreferences: {},
  session: {
    defaultSession: {
      setPermissionCheckHandler: (handler: typeof handlers.check) => (handlers.check = handler),
      setPermissionRequestHandler: (handler: typeof handlers.request) =>
        (handlers.request = handler),
    },
  },
}))

const { setupMediaPermissions } = await import('../../electron/main/microphone')

const APP = 'kchess://app/index.html'
const OTHER = 'https://example.com/'
setupMediaPermissions((url) => url.startsWith('kchess://app/'))
const contents = (url: string) => ({ getURL: () => url })

function request(permission: string, url: string, details: Record<string, unknown> = {}) {
  const callback = vi.fn()
  handlers.request!(contents(url), permission, callback, {
    requestingUrl: url,
    isMainFrame: true,
    ...details,
  })
  return callback.mock.calls[0]?.[0]
}

it('lets the app’s own page write to the clipboard so the Copy buttons work', () => {
  expect(handlers.check!(contents(APP), 'clipboard-sanitized-write', APP, {})).toBe(true)
  expect(request('clipboard-sanitized-write', APP)).toBe(true)
})

it('keeps clipboard reads and foreign pages denied', () => {
  expect(handlers.check!(contents(APP), 'clipboard-read', APP, {})).toBe(false)
  expect(request('clipboard-read', APP)).toBe(false)
  expect(handlers.check!(contents(OTHER), 'clipboard-sanitized-write', OTHER, {})).toBe(false)
  expect(request('clipboard-sanitized-write', OTHER)).toBe(false)
  expect(request('clipboard-sanitized-write', APP, { isMainFrame: false })).toBe(false)
})

it('still grants audio capture only', () => {
  expect(handlers.check!(contents(APP), 'media', APP, { mediaType: 'audio' })).toBe(true)
  expect(handlers.check!(contents(APP), 'media', APP, { mediaType: 'video' })).toBe(false)
  expect(request('media', APP, { mediaTypes: ['audio'] })).toBe(true)
  expect(request('media', APP, { mediaTypes: ['audio', 'video'] })).toBe(false)
})
