import { readFileSync, renameSync, writeFileSync } from 'node:fs'
import type { BrowserWindow, Rectangle } from 'electron'
import { logDebug, logWarn } from '@kchess/native/logger'

/** How the window was left: its normal (unmaximized) bounds and its mode. */
export interface WindowState {
  x: number
  y: number
  width: number
  height: number
  maximized: boolean
  fullScreen: boolean
}

export const DEFAULT_SIZE = { width: 1320, height: 880 } as const
export const MIN_SIZE = { width: 640, height: 620 } as const
/** The top strip that must stay on a screen so the window can still be grabbed and moved. */
const GRAB_HEIGHT = 52
const GRAB_WIDTH = 120
const SAVE_DELAY_MS = 500

const finite = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value)

/** A saved state, or null when the file is missing, from another version, or damaged. */
export function parseWindowState(raw: unknown): WindowState | null {
  if (!raw || typeof raw !== 'object') return null
  const { x, y, width, height, maximized, fullScreen } = raw as Record<string, unknown>
  if (!finite(x) || !finite(y) || !finite(width) || !finite(height)) return null
  if (width <= 0 || height <= 0) return null
  return {
    x: Math.round(x),
    y: Math.round(y),
    width: Math.round(width),
    height: Math.round(height),
    maximized: maximized === true,
    fullScreen: fullScreen === true,
  }
}

/** The display work area that holds the window's title strip, if any still does. */
function grabbableOn(bounds: Rectangle, areas: readonly Rectangle[]): Rectangle | undefined {
  return areas.find((area) => {
    const overlap =
      Math.min(bounds.x + bounds.width, area.x + area.width) - Math.max(bounds.x, area.x)
    return (
      overlap >= Math.min(GRAB_WIDTH, bounds.width) &&
      bounds.y >= area.y &&
      bounds.y + GRAB_HEIGHT <= area.y + area.height
    )
  })
}

/**
 * Where to open the window. A saved position that no connected display can show (a monitor was
 * unplugged, or the resolution dropped) falls back to the default size, centred by the system.
 * Sizes never exceed the display they land on, nor drop below the window's minimum.
 */
export function restoredBounds(
  saved: WindowState | null,
  areas: readonly Rectangle[],
  primary: Rectangle,
): Partial<Rectangle> & { width: number; height: number } {
  const fit = (size: { width: number; height: number }, area: Rectangle) => ({
    width: Math.max(MIN_SIZE.width, Math.min(size.width, area.width)),
    height: Math.max(MIN_SIZE.height, Math.min(size.height, area.height)),
  })
  const area = saved && grabbableOn(saved, areas)
  if (!saved || !area) return fit(DEFAULT_SIZE, primary)
  const size = fit(saved, area)
  // Shrinking to fit can push the far edge off the display; slide back so it is all visible.
  const x = Math.max(area.x, Math.min(saved.x, area.x + area.width - size.width))
  const y = Math.max(area.y, Math.min(saved.y, area.y + area.height - size.height))
  return { x, y, ...size }
}

export function readWindowState(file: string): WindowState | null {
  try {
    return parseWindowState(JSON.parse(readFileSync(file, 'utf8')))
  } catch (cause) {
    // A first launch has no file; anything else just means opening at the default size.
    if ((cause as NodeJS.ErrnoException).code !== 'ENOENT')
      logWarn('window', 'Window state unreadable; using the default size:', cause)
    return null
  }
}

function writeWindowState(file: string, state: WindowState): void {
  try {
    const temp = `${file}.tmp`
    writeFileSync(temp, `${JSON.stringify(state)}\n`)
    renameSync(temp, file)
  } catch (cause) {
    logWarn('window', 'Window state not saved:', cause)
  }
}

/** Puts a new window back into the mode it was left in. */
export function applyWindowMode(window: BrowserWindow, saved: WindowState | null): void {
  if (saved?.fullScreen) window.setFullScreen(true)
  else if (saved?.maximized) window.maximize()
}

/** Saves the window's bounds and mode shortly after each change, and once more as it closes. */
export function trackWindowState(window: BrowserWindow, file: string): void {
  let timer: ReturnType<typeof setTimeout> | undefined
  const save = (): void => {
    clearTimeout(timer)
    timer = undefined
    // A minimized window reports nothing useful; the last saved state stands.
    if (window.isDestroyed() || window.isMinimized()) return
    const bounds = window.getNormalBounds()
    writeWindowState(file, {
      ...bounds,
      maximized: window.isMaximized(),
      fullScreen: window.isFullScreen(),
    })
    logDebug('window', 'Window state saved')
  }
  const later = (): void => {
    clearTimeout(timer)
    timer = setTimeout(save, SAVE_DELAY_MS)
  }
  for (const event of [
    'resize',
    'move',
    'maximize',
    'unmaximize',
    'enter-full-screen',
    'leave-full-screen',
  ] as const)
    window.on(event as 'resize', later)
  window.on('close', save)
}
