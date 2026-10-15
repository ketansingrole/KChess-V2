import { EventEmitter } from 'node:events'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BrowserWindow } from 'electron'
import {
  applyWindowMode,
  DEFAULT_SIZE,
  parseWindowState,
  readWindowState,
  restoredBounds,
  trackWindowState,
  type WindowState,
} from '../../electron/main/windowState'

const laptop = { x: 0, y: 25, width: 1512, height: 920 }
const external = { x: 1512, y: 0, width: 2560, height: 1415 }
const state = (overrides: Partial<WindowState> = {}): WindowState => ({
  x: 100,
  y: 80,
  width: 1200,
  height: 800,
  maximized: false,
  fullScreen: false,
  ...overrides,
})

describe('restoredBounds', () => {
  it('opens at the default size on a first launch', () => {
    expect(restoredBounds(null, [laptop], laptop)).toEqual(DEFAULT_SIZE)
  })

  it('reopens where the window was left', () => {
    expect(restoredBounds(state(), [laptop], laptop)).toEqual({
      x: 100,
      y: 80,
      width: 1200,
      height: 800,
    })
  })

  it('reopens on the external display it was left on', () => {
    const saved = state({ x: 2000, y: 100, width: 1800, height: 1100 })
    expect(restoredBounds(saved, [laptop, external], laptop)).toEqual({
      x: 2000,
      y: 100,
      width: 1800,
      height: 1100,
    })
  })

  it('falls back to the default size when the display it was on is unplugged', () => {
    const saved = state({ x: 2000, y: 100, width: 1800, height: 1100 })
    expect(restoredBounds(saved, [laptop], laptop)).toEqual(DEFAULT_SIZE)
  })

  it('falls back when the title strip is above or below every display', () => {
    expect(restoredBounds(state({ y: -300 }), [laptop], laptop)).toEqual(DEFAULT_SIZE)
    expect(restoredBounds(state({ y: 900 }), [laptop], laptop)).toEqual(DEFAULT_SIZE)
  })

  it('shrinks a window larger than its display and keeps it all on screen', () => {
    const saved = state({ x: 600, y: 300, width: 2400, height: 1400 })
    expect(restoredBounds(saved, [laptop], laptop)).toEqual({
      x: 0,
      y: 25,
      width: 1512,
      height: 920,
    })
  })

  it('never opens below the minimum size', () => {
    expect(restoredBounds(state({ width: 200, height: 150 }), [laptop], laptop)).toMatchObject({
      width: 640,
      height: 620,
    })
  })
})

describe('saved state file', () => {
  let directory: string
  beforeEach(async () => {
    directory = await mkdtemp(join(tmpdir(), 'kchess-window-'))
  })
  afterEach(async () => {
    await rm(directory, { recursive: true, force: true })
  })

  it('ignores a missing, damaged or foreign file', async () => {
    const file = join(directory, 'window-state.json')
    expect(readWindowState(file)).toBeNull()
    await writeFile(file, '{"x": 1,')
    expect(readWindowState(file)).toBeNull()
    expect(parseWindowState({ x: '1', y: 0, width: 10, height: 10 })).toBeNull()
    expect(parseWindowState({ x: 0, y: 0, width: 0, height: 10 })).toBeNull()
  })

  it('saves the normal bounds and mode, and skips a minimized window', async () => {
    vi.useFakeTimers()
    try {
      const file = join(directory, 'window-state.json')
      const fake = Object.assign(new EventEmitter(), {
        isDestroyed: () => false,
        isMinimized: vi.fn(() => false),
        isMaximized: () => true,
        isFullScreen: () => false,
        getNormalBounds: () => ({ x: 40, y: 60, width: 1100, height: 760 }),
      })
      trackWindowState(fake as unknown as BrowserWindow, file)
      fake.emit('maximize')
      vi.advanceTimersByTime(600)
      expect(JSON.parse(await readFile(file, 'utf8'))).toEqual({
        x: 40,
        y: 60,
        width: 1100,
        height: 760,
        maximized: true,
        fullScreen: false,
      })

      fake.isMinimized.mockReturnValue(true)
      await writeFile(file, '{}')
      fake.emit('close')
      expect(await readFile(file, 'utf8')).toBe('{}')
    } finally {
      vi.useRealTimers()
    }
  })

  it('puts the window back into full screen or maximized', () => {
    const window = { setFullScreen: vi.fn(), maximize: vi.fn() }
    applyWindowMode(
      window as unknown as BrowserWindow,
      state({ fullScreen: true, maximized: true }),
    )
    expect(window.setFullScreen).toHaveBeenCalledWith(true)
    expect(window.maximize).not.toHaveBeenCalled()
    applyWindowMode(window as unknown as BrowserWindow, state({ maximized: true }))
    expect(window.maximize).toHaveBeenCalledOnce()
  })
})
