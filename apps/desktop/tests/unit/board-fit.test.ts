import { expect, it } from 'vitest'
import { fitBoard } from '../../app/utils/boardFit'

type Box = { top: number; height: number; width?: number }

function place(element: HTMLElement, { top, height, width = 100 }: Box): HTMLElement {
  element.getBoundingClientRect = () =>
    ({ top, height, bottom: top + height, left: 0, right: width, width, x: 0, y: top }) as DOMRect
  Object.defineProperty(element, 'offsetWidth', { value: width })
  Object.defineProperty(element, 'offsetHeight', { value: height })
  return element
}
function element(tag: string, className = '', parent?: HTMLElement): HTMLElement {
  const node = document.createElement(tag)
  node.className = className
  parent?.append(node)
  return node
}

/*
 * Each board page used to reserve a hand-written amount of height for everything but the board
 * (300px by default). Pages with less around the board left a gap under it (78px on Coordinates,
 * 94px on Analysis, 95px on a Lichess game), and a board wrapper holding rows under the board hid
 * them from the count, so those pages scrolled instead.
 */
it('reserves exactly the height around the board', () => {
  const scroller = element('main')
  const page = place(element('div', 'page', scroller), { top: 52, height: 848 })
  place(element('header', '', page), { top: 76, height: 40 })
  const layout = place(element('div', 'play-layout', page), { top: 128, height: 732 })
  const column = place(element('div', 'board-stack', layout), { top: 128, height: 732 })
  place(element('div', 'player-line', column), { top: 128, height: 36 })
  // The wrapper also holds the typed-move row; only the squares count as the board.
  const host = place(element('div', 'cg-host', column), { top: 174, height: 686 })
  place(element('cg-container', '', host), { top: 174, height: 640, width: 640 })
  place(element('cg-container', '', layout), { top: 0, height: 120, width: 120 })

  fitBoard(layout)

  // 128 above the column + 92 in it beside the board + 40 of page padding under the layout.
  expect(layout.style.getPropertyValue('--board-chrome')).toBe('260px')
})

it('leaves a layout without a shown board alone', () => {
  const page = place(element('div', 'page', element('main')), { top: 52, height: 848 })
  const layout = place(element('div', 'play-layout', page), { top: 128, height: 0 })
  place(element('cg-container', '', element('div', '', layout)), { top: 128, height: 0, width: 0 })

  fitBoard(layout)

  expect(layout.style.getPropertyValue('--board-chrome')).toBe('')
})

/*
 * The grid stretches the board's column to a taller side panel (broadcast engine lines). Counting
 * that stretch as chrome shrank the board, which kept the stretch, so switching the evaluation bar
 * off left the board at its 260px floor in a 1024×700 window.
 */
it('ignores the stretch of a column beside a taller side panel', () => {
  // The page ends 40px under the layout, as above.
  const page = place(element('div', 'page', element('main')), { top: 52, height: 1016 })
  const layout = place(element('div', 'play-layout', page), { top: 128, height: 900 })
  const column = place(element('div', 'board-stack', layout), { top: 128, height: 900 })
  place(element('div', 'player-line', column), { top: 128, height: 36 })
  const host = place(element('div', 'broadcast-board', column), { top: 174, height: 400 })
  place(element('cg-container', '', host), { top: 174, height: 400, width: 400 })
  place(element('div', 'player-line', column), { top: 584, height: 36 })
  place(element('div', 'hidden-row', column), { top: 0, height: 0, width: 0 })
  place(element('aside', 'side-panel', layout), { top: 128, height: 900 })

  fitBoard(layout)

  // 128 above + 92 of player lines and gaps + 40 below; not the 400px of stretch under them.
  expect(layout.style.getPropertyValue('--board-chrome')).toBe('260px')
})
