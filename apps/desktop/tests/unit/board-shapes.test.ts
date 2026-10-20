import { expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import type { Api } from '@lichess-org/chessground/api'
import type { DrawShape } from '@lichess-org/chessground/draw'
import { INITIAL_FEN } from '@kchess/rules/position'
import ChessBoard from '../../app/components/ChessBoard.vue'

const grounds: Api[] = []
vi.mock('@lichess-org/chessground', async (original) => {
  const actual = await original<typeof import('@lichess-org/chessground')>()
  return {
    ...actual,
    Chessground: (...args: Parameters<typeof actual.Chessground>) => {
      const ground = actual.Chessground(...args)
      grounds.push(ground)
      return ground
    },
  }
})

// Engine arrows replaced the player's own right-click drawings once a shapes change called
// `setShapes`: the app's arrows are auto shapes, and only those may follow the prop.
it('updates the app arrows without touching the player drawings or the position', async () => {
  const board = mount(ChessBoard, { props: { fen: INITIAL_FEN }, attachTo: document.body })
  await flushPromises()
  const ground = grounds.at(-1)!
  const drawn: DrawShape = { orig: 'a2', dest: 'a4', brush: 'green' }
  ground.setShapes([drawn])
  const set = vi.spyOn(ground, 'set')

  const arrow: DrawShape = { orig: 'e2', dest: 'e4', brush: 'paleBlue' }
  await board.setProps({ shapes: [arrow] })
  expect(ground.state.drawable.autoShapes).toEqual([arrow])
  expect(ground.state.drawable.shapes).toEqual([drawn])
  expect(set).not.toHaveBeenCalled()

  await board.setProps({ shapes: [] })
  expect(ground.state.drawable.autoShapes).toEqual([])
  expect(ground.state.drawable.shapes).toEqual([drawn])
  board.unmount()
})
