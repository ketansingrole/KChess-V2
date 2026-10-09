import { describe, expect, it } from 'vitest'
import { resolveUpdateCapabilities } from '../../electron/main/setupAppUpdates'

describe('update capability matrix', () => {
  it('disables checks in development builds', () => {
    const capabilities = resolveUpdateCapabilities({
      dev: true,
      hasFeed: false,
      platform: 'darwin',
      macSigned: true,
    })
    expect(capabilities).toMatchObject({ canCheck: false, canInstall: false })
    expect(capabilities.reason).toMatch(/Development builds/)
  })

  it('allows signed Mac, Windows and packaged Linux builds to self-install', () => {
    for (const host of [
      { dev: false, hasFeed: true, platform: 'darwin', macSigned: true },
      { dev: false, hasFeed: true, platform: 'win32', macSigned: false },
      { dev: false, hasFeed: true, platform: 'linux', packageType: 'deb', macSigned: false },
      { dev: false, hasFeed: true, platform: 'linux', appImage: '/app.AppImage', macSigned: false },
    ] as const) {
      const capabilities = resolveUpdateCapabilities(host)
      expect(capabilities.canCheck).toBe(true)
      expect(capabilities.canInstall).toBe(true)
      expect(capabilities.reason).toBeUndefined()
    }
  })

  it('checks but never self-installs unsigned Mac builds', () => {
    const capabilities = resolveUpdateCapabilities({
      dev: false,
      hasFeed: true,
      platform: 'darwin',
      macSigned: false,
    })
    expect(capabilities).toMatchObject({ canCheck: true, canInstall: false })
    expect(capabilities.reason).toMatch(/Developer ID/)
  })

  it('refuses unpacked Linux directories and feed-less installs', () => {
    const unpacked = resolveUpdateCapabilities({
      dev: false,
      hasFeed: true,
      platform: 'linux',
      macSigned: false,
    })
    expect(unpacked.canCheck).toBe(false)
    const noFeed = resolveUpdateCapabilities({
      dev: false,
      hasFeed: false,
      platform: 'win32',
      macSigned: false,
    })
    expect(noFeed.canCheck).toBe(false)
    expect(noFeed.reason).toMatch(/cannot update itself/)
  })
})
