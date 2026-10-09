import { describe, expect, it } from 'vitest'
import { APP_ICON_SVG } from '../../src/services/appIconSvg'

describe('APP_ICON_SVG', () => {
  it('is a non-empty svg with a viewBox and no scripts', () => {
    expect(typeof APP_ICON_SVG).toBe('string')
    expect(APP_ICON_SVG.length).toBeGreaterThan(0)
    expect(APP_ICON_SVG.startsWith('<svg')).toBe(true)
    expect(APP_ICON_SVG).toContain('viewBox')
    expect(APP_ICON_SVG.toLowerCase()).not.toContain('<script')
  })
})
