import type { Directive } from 'vue'

/**
 * Board layouts size the board as `100dvh - var(--board-chrome)`: the window height less everything
 * in the board's column that is not the board (the topbar, headers and tabs above it, player lines
 * and inputs around it, rows below the layout and the page's bottom padding). Measured here rather than written per page, so
 * a taller header or another row never leaves a gap under the board or scrolls the page.
 */
export function fitBoard(layout: HTMLElement): void {
  // The squares themselves (a board's wrapper can hold rows under it); the main board is the
  // largest one, as layouts can also hold thumbnails.
  let board: HTMLElement | undefined
  for (const host of layout.querySelectorAll<HTMLElement>('cg-container'))
    if (!board || host.offsetWidth > board.offsetWidth) board = host
  const page = layout.closest<HTMLElement>('.page')
  if (!board || !page || !board.offsetHeight) return
  let column: HTMLElement = board
  while (column.parentElement && column.parentElement !== layout) column = column.parentElement
  if (column.parentElement !== layout) return
  const scrolled = page.parentElement?.scrollTop ?? 0
  // The column's content, not its box: the grid stretches the column to a taller side panel, and
  // counting that stretch as chrome would shrink the board, which keeps the stretch, for good.
  // Hidden rows (no box) would otherwise pull the top to the viewport's edge.
  const rows = (column === board ? [board] : [...column.children])
    .map((row) => row.getBoundingClientRect())
    .filter((row) => row.width || row.height)
  const top = Math.min(...rows.map((row) => row.top))
  const bottom = Math.max(...rows.map((row) => row.bottom))
  const above = top + scrolled
  const around = bottom - top - board.getBoundingClientRect().height
  // Rows under the whole layout (and the page's bottom padding); a side panel taller than the
  // board stays inside the layout, so it scrolls rather than shrinking the board to nothing.
  const below = page.getBoundingClientRect().bottom - layout.getBoundingClientRect().bottom
  const chrome = `${Math.ceil(above + around + below)}px`
  if (layout.style.getPropertyValue('--board-chrome') !== chrome)
    layout.style.setProperty('--board-chrome', chrome)
}

const observers = new WeakMap<HTMLElement, { observer: ResizeObserver; frame: number }>()

/** `v-board-fit` on a board layout keeps its `--board-chrome` measured as the page changes. */
export const boardFit: Directive<HTMLElement> = {
  mounted(layout) {
    fitBoard(layout)
    const entry = {
      frame: 0,
      // The page resizes with the window and with anything above the board; the layout with its rows.
      observer: new ResizeObserver(() => {
        cancelAnimationFrame(entry.frame)
        entry.frame = requestAnimationFrame(() => fitBoard(layout))
      }),
    }
    entry.observer.observe(layout)
    const page = layout.closest<HTMLElement>('.page')
    if (page) entry.observer.observe(page)
    observers.set(layout, entry)
  },
  unmounted(layout) {
    const entry = observers.get(layout)
    if (!entry) return
    cancelAnimationFrame(entry.frame)
    entry.observer.disconnect()
    observers.delete(layout)
  },
}
