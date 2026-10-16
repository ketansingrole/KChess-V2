import { describe, expect, it } from 'vitest'
import { autoZoomPercent, isInterfaceZoom, stepZoom } from '../../contracts/interfaceZoom'
import { zoomShortcut } from '../../electron/main/interfaceZoom'

describe('interface zoom', () => {
  it('scales up only for workspaces larger than 1440p', () => {
    expect(autoZoomPercent({ width: 1280, height: 760 })).toBe(100)
    expect(autoZoomPercent({ width: 1920, height: 1040 })).toBe(100)
    expect(autoZoomPercent({ width: 2560, height: 1400 })).toBe(100)
    // An ultrawide 1440p screen is wider, not denser.
    expect(autoZoomPercent({ width: 3440, height: 1400 })).toBe(100)
    // 4K, 5K and 8K at 100% OS scaling, with a taskbar or menu bar.
    expect(autoZoomPercent({ width: 3840, height: 2120 })).toBe(150)
    expect(autoZoomPercent({ width: 2160, height: 3800 })).toBe(150)
    expect(autoZoomPercent({ width: 5120, height: 2850 })).toBe(200)
    expect(autoZoomPercent({ width: 7680, height: 4280 })).toBe(300)
    // 4K at 150% OS scaling is already a 1440p workspace.
    expect(autoZoomPercent({ width: 2560, height: 1410 })).toBe(100)
  })

  it('steps between zoom levels and holds at the ends', () => {
    expect(stepZoom(100, 1)).toBe(110)
    expect(stepZoom(100, -1)).toBe(90)
    expect(stepZoom(300, 1)).toBe(300)
    expect(stepZoom(50, -1)).toBe(50)
    expect(stepZoom(133, 1)).toBe(150)
  })

  it('accepts only auto and the zoom steps', () => {
    expect(isInterfaceZoom('auto')).toBe(true)
    expect(isInterfaceZoom(125)).toBe(true)
    expect(isInterfaceZoom(133)).toBe(false)
    expect(isInterfaceZoom('125')).toBe(false)
  })

  it('reads zoom shortcuts with the platform modifier, with or without Shift', () => {
    const key = (
      key: string,
      extra: Partial<{ control: boolean; meta: boolean; alt: boolean; code: string }> = {},
    ) => ({
      type: 'keyDown' as const,
      key,
      code: extra.code ?? '',
      control: extra.control ?? false,
      meta: extra.meta ?? false,
      alt: extra.alt ?? false,
    })
    expect(zoomShortcut(key('=', { control: true }), false)).toBe(1)
    expect(zoomShortcut(key('+', { control: true }), false)).toBe(1)
    expect(zoomShortcut(key('-', { control: true }), false)).toBe(-1)
    expect(zoomShortcut(key('0', { control: true }), false)).toBe(0)
    expect(zoomShortcut(key('+', { control: true, code: 'NumpadAdd' }), false)).toBe(1)
    expect(zoomShortcut(key('=', { meta: true }), true)).toBe(1)
    // Wrong modifier for the platform, Alt chords and plain keys are left to the page.
    expect(zoomShortcut(key('=', { meta: true }), false)).toBeNull()
    expect(zoomShortcut(key('=', { control: true }), true)).toBeNull()
    expect(zoomShortcut(key('=', { control: true, alt: true }), false)).toBeNull()
    expect(zoomShortcut(key('='), false)).toBeNull()
    expect(
      zoomShortcut({ ...key('=', { control: true }), type: 'keyUp' as const }, false),
    ).toBeNull()
  })
})
