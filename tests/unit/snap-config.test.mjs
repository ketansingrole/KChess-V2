import { createRequire } from 'node:module'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const require = createRequire(join(process.cwd(), 'package.json'))
const builderRequire = createRequire(require.resolve('electron-builder'))
const { SnapCore24 } = builderRequire('app-builder-lib/out/targets/snap/core24.js')
const config = require('./tooling/snap-config.cjs')

describe('Snap package descriptor', () => {
  it('keeps the registered identity and discovers the source desktop entry before packing', async () => {
    const core = new SnapCore24(
      {
        executableName: config.linux.executableName,
        appInfo: { version: '2026.10.9', productName: 'KChess' },
      },
      {
        isElectronVersionGreaterOrEqualThan: () => true,
        getDescription: () => 'Chess',
        getDesktopFileName: (name) => name,
        icons: Promise.resolve(),
      },
      config.snapcraft.core24,
    )
    const snap = await core.createDescriptor(1)
    expect(snap.name).toBe('kchess')
    expect(snap.confinement).toBe('strict')
    expect(snap.apps.kchess.extensions).toEqual(['gnome'])
    // snap/gui/kchess.desktop is discovered automatically. meta/gui is not present at build time.
    expect(snap.apps.kchess).not.toHaveProperty('desktop')
    // allow-sandbox needs manual Store review, so Chromium's own sandbox is off inside the Snap.
    expect(snap.plugs?.['browser-support']).toBeUndefined()
    expect(snap.apps.kchess.plugs).toContain('browser-support')
    expect(core.commandArgs).toContain('--no-sandbox')
  })
})
