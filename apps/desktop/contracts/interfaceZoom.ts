/** Interface zoom: the whole window scaled together, like a browser's page zoom. */
export const ZOOM_PERCENTS = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300] as const
export type ZoomPercent = (typeof ZOOM_PERCENTS)[number]
/** `auto` follows the display the window is on; a percent is the user's own choice. */
export type InterfaceZoom = 'auto' | ZoomPercent
export interface InterfaceZoomState {
  setting: InterfaceZoom
  /** The zoom in effect: the setting, or what `auto` chose for the current display. */
  percent: ZoomPercent
  /** What `auto` would choose on the current display. */
  autoPercent: ZoomPercent
}

/*
 * The workspace a 100% interface is designed for, in device-independent pixels (a display's
 * work area at its OS scale). A 4K screen left at 100% OS scaling is 3840×2160 of them, so the
 * interface would draw at two thirds of its intended size; `auto` makes up the difference.
 * The short side allows for a taskbar or menu bar.
 */
const DESIGN_LONG_SIDE = 2560
const DESIGN_SHORT_SIDE = 1360

export function isInterfaceZoom(value: unknown): value is InterfaceZoom {
  return value === 'auto' || ZOOM_PERCENTS.includes(value as ZoomPercent)
}

/**
 * The zoom `auto` picks for a work area: 100% up to a 1440p-sized workspace, then the largest
 * step that keeps the interface's proportion of the screen (150% on 4K, 200% on 5K, 300% on 8K
 * at 100% OS scaling). Either orientation counts the same; never below 100%.
 */
export function autoZoomPercent(area: { width: number; height: number }): ZoomPercent {
  const long = Math.max(area.width, area.height)
  const short = Math.min(area.width, area.height)
  const ratio = Math.min(long / DESIGN_LONG_SIDE, short / DESIGN_SHORT_SIDE) * 100
  let chosen: ZoomPercent = 100
  // A few percent of slack: a taskbar along the long side (a portrait screen) must not cost a step.
  for (const step of ZOOM_PERCENTS) if (step >= 100 && step <= ratio + 3) chosen = step
  return chosen
}

/** The next zoom step in `direction` from `percent`, held at the smallest and largest. */
export function stepZoom(percent: number, direction: 1 | -1): ZoomPercent {
  const largest: ZoomPercent = 300
  if (direction > 0) return ZOOM_PERCENTS.find((step) => step > percent) ?? largest
  return [...ZOOM_PERCENTS].reverse().find((step) => step < percent) ?? ZOOM_PERCENTS[0]
}

export function zoomState(setting: InterfaceZoom, autoPercent: ZoomPercent): InterfaceZoomState {
  return { setting, percent: setting === 'auto' ? autoPercent : setting, autoPercent }
}
