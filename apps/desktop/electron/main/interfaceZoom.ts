import { readFileSync, renameSync, writeFileSync } from 'node:fs'
import type { BrowserWindow, Input, Rectangle } from 'electron'
import { logDebug, logWarn } from '@kchess/native/logger'
import {
  autoZoomPercent,
  isInterfaceZoom,
  stepZoom,
  zoomState,
  type InterfaceZoom,
  type InterfaceZoomState,
} from '../../contracts/interfaceZoom'

/** The renderer's topbar height at 100%, which the macOS traffic lights centre on (main.css). */
const TOPBAR_HEIGHT = 52
const TRAFFIC_LIGHT_X = 12
const TRAFFIC_LIGHT_SIZE = 12
/** Trackpads send a burst of Ctrl+wheel events per gesture; take one step per burst. */
const WHEEL_STEP_MS = 160

export function readInterfaceZoom(file: string): InterfaceZoom {
  try {
    const { zoom } = JSON.parse(readFileSync(file, 'utf8')) as { zoom?: unknown }
    return isInterfaceZoom(zoom) ? zoom : 'auto'
  } catch (cause) {
    if ((cause as NodeJS.ErrnoException).code !== 'ENOENT')
      logWarn('zoom', 'Interface zoom unreadable; using Auto:', cause)
    return 'auto'
  }
}

function writeInterfaceZoom(file: string, zoom: InterfaceZoom): void {
  try {
    const temp = `${file}.tmp`
    writeFileSync(temp, `${JSON.stringify({ zoom })}\n`)
    renameSync(temp, file)
  } catch (cause) {
    logWarn('zoom', 'Interface zoom not saved:', cause)
  }
}

/** What a zoom shortcut asks for: one step either way, or back to the default. */
export function zoomShortcut(
  input: Pick<Input, 'type' | 'key' | 'code' | 'control' | 'meta' | 'alt'>,
  mac: boolean,
): 1 | -1 | 0 | null {
  if (input.type !== 'keyDown' || input.alt) return null
  if (mac ? !input.meta || input.control : !input.control || input.meta) return null
  if (input.key === '=' || input.key === '+' || input.code === 'NumpadAdd') return 1
  if (input.key === '-' || input.key === '_' || input.code === 'NumpadSubtract') return -1
  if (input.key === '0' || input.code === 'Numpad0') return 0
  return null
}

interface ZoomHost {
  /** The work area of the display that shows most of `bounds`. */
  workAreaOf(bounds: Rectangle): Rectangle
  onDisplaysChanged(listener: () => void): () => void
}

/**
 * Owns the interface zoom: the saved setting, the factor applied to the window, the keyboard and
 * Ctrl+wheel shortcuts, and `auto` following the window across displays.
 */
export class InterfaceZoomController {
  private setting: InterfaceZoom
  private window: BrowserWindow | null = null
  private lastWheel = 0
  private readonly listeners = new Set<(state: InterfaceZoomState) => void>()

  constructor(
    private readonly file: string,
    private readonly host: ZoomHost,
    private readonly mac = process.platform === 'darwin',
  ) {
    this.setting = readInterfaceZoom(file)
  }

  state(): InterfaceZoomState {
    const window = this.window && !this.window.isDestroyed() ? this.window : null
    const auto = window ? autoZoomPercent(this.host.workAreaOf(window.getBounds())) : 100
    return zoomState(this.setting, auto)
  }

  onChange(listener: (state: InterfaceZoomState) => void): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  set(setting: InterfaceZoom): InterfaceZoomState {
    if (setting !== this.setting) {
      this.setting = setting
      writeInterfaceZoom(this.file, setting)
    }
    return this.apply()
  }

  /** One step larger or smaller than what is shown, or 0 for the default (Auto). */
  step(direction: 1 | -1 | 0): InterfaceZoomState {
    return this.set(direction === 0 ? 'auto' : stepZoom(this.state().percent, direction))
  }

  attach(window: BrowserWindow): void {
    this.window = window
    const { webContents } = window
    webContents.on('before-input-event', (event, input) => {
      const direction = zoomShortcut(input, this.mac)
      if (direction === null) return
      // Also keeps Chromium's and the default menu's own zoom from running with ours.
      event.preventDefault()
      this.step(direction)
    })
    webContents.on('zoom-changed', (_event, direction) => {
      const now = Date.now()
      if (now - this.lastWheel < WHEEL_STEP_MS) return
      this.lastWheel = now
      this.step(direction === 'in' ? 1 : -1)
    })
    // Chromium keeps zoom per origin; a reload or the first load must show the saved one.
    webContents.on('did-finish-load', () => this.apply())
    // Auto follows the display: re-check once a move ends, or when a display changes.
    let shown = this.state().percent
    const follow = (): void => {
      if (this.setting !== 'auto' || window.isDestroyed()) return
      const percent = this.state().percent
      if (percent === shown) return
      shown = percent
      this.apply()
    }
    window.on('moved', follow)
    const stopDisplays = this.host.onDisplaysChanged(follow)
    const stopShown = this.onChange((state) => {
      shown = state.percent
    })
    window.on('closed', () => {
      stopDisplays()
      stopShown()
      if (this.window === window) this.window = null
    })
    this.apply()
  }

  private apply(): InterfaceZoomState {
    const state = this.state()
    const window = this.window
    if (window && !window.isDestroyed()) {
      const factor = state.percent / 100
      window.webContents.setZoomFactor(factor)
      if (this.mac) {
        // The traffic lights are native and do not zoom: keep them centred on the topbar.
        const y = Math.round((TOPBAR_HEIGHT * factor - TRAFFIC_LIGHT_SIZE) / 2)
        window.setWindowButtonPosition({ x: TRAFFIC_LIGHT_X, y: Math.max(4, y) })
      }
      logDebug('zoom', 'Interface zoom applied:', state.setting, state.percent)
    }
    for (const listener of this.listeners) listener(state)
    return state
  }
}
