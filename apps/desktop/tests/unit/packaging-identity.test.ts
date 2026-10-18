import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { APP_ID, APP_NAME, appUserModelId } from '../../electron/main/appIdentity'

const manifest = JSON.parse(readFileSync(resolve(__dirname, '../../package.json'), 'utf8')) as {
  name: string
  build: {
    appId: string
    productName: string
    nsis: { guid?: string }
    linux: { executableName?: string }
    deb: { fpm?: string[] }
  }
}

describe('packaging identity', () => {
  it('names the app KChess without its framework', () => {
    expect(manifest.name).toBe('kchess')
    expect(manifest.build.appId).toBe('com.kchess.app')
    expect(manifest.build.productName).toBe('KChess')
    expect(manifest.build.linux.executableName).toBe('kchess')
  })

  it('upgrades installs made under the former com.kchess.electron identity', () => {
    // The NSIS GUID was derived from the former appId. Changing it would make Windows treat the
    // next update as a different app and leave the previous installation beside it.
    expect(manifest.build.nsis.guid).toBe('6bb66f38-a1a5-5ec8-b24f-c899466885ca')
    // The deb package was named after the former package name.
    expect(manifest.build.deb.fpm).toEqual(
      expect.arrayContaining(['--replaces=kchess-electron', '--conflicts=kchess-electron']),
    )
  })

  it('runs under the AppUserModelID that the installer stamps on its shortcuts', () => {
    // A mismatch makes Windows group the running app and its notifications apart from the
    // Start-menu shortcut, under Electron's fallback ID.
    expect(APP_ID).toBe(manifest.build.appId)
    expect(APP_NAME).toBe(manifest.build.productName)
    expect(appUserModelId(true, 'C:\\KChess\\KChess.exe')).toBe(manifest.build.appId)
    expect(appUserModelId(false, 'C:\\dev\\KChess.exe')).toBe('C:\\dev\\KChess.exe')
  })
})
