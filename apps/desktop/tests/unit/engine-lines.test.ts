import { mount } from '@vue/test-utils'
import { expect, it } from 'vitest'
import { INITIAL_FEN } from '@kchess/rules/position'
import EngineLines from '../../app/components/EngineLines.vue'
import { addMove, newTree, pvSan } from '@kchess/rules/analysisTree'
import type { EngineLine } from '@kchess/contracts/types'

const line: EngineLine = { rank: 1, depth: 18, cp: 30, pv: ['e2e4', 'e7e5', 'g1f3'] }
it('shares hover and keyboard previews, keeps them stable during updates, and clears them on position change', async () => {
  const wrapper = mount(EngineLines, {
    attachTo: document.body,
    props: { fen: INITIAL_FEN, lines: [line], orientation: 'black', pending: 3 },
    global: {
      stubs: {
        ChessBoard: {
          props: ['fen', 'orientation', 'lastMove'],
          template: '<div class="preview-board" :data-fen="fen" :data-orientation="orientation" />',
        },
      },
    },
  })
  expect(wrapper.findAll('.pv-row.pending')).toHaveLength(2)
  const move = wrapper.findAll('.pv-move')[1]!
  await move.trigger('mouseenter')
  const fen = pvSan(INITIAL_FEN, line.pv)[1]!.fen
  expect(document.querySelector('.preview-board')?.getAttribute('data-fen')).toBe(fen)
  expect(document.querySelector('.preview-board')?.getAttribute('data-orientation')).toBe('black')
  await wrapper.setProps({ lines: [{ ...line, pv: ['d2d4', 'd7d5'] }] })
  expect(document.querySelector('.preview-board')?.getAttribute('data-fen')).toBe(fen)
  await wrapper.setProps({ fen })
  expect(document.querySelector('.pv-preview')).toBeNull()
  await wrapper.setProps({ fen: INITIAL_FEN, lines: [line] })
  await wrapper.findAll('.pv-move')[0]!.trigger('focus')
  expect(document.querySelector('.pv-preview')).not.toBeNull()
  await wrapper.findAll('.pv-move')[0]!.trigger('blur')
  expect(document.querySelector('.pv-preview')).toBeNull()
  await wrapper.findAll('.pv-move')[2]!.trigger('click')
  expect(wrapper.emitted('select')?.[0]).toEqual([line.pv, 2])
  wrapper.unmount()
  expect(document.querySelector('.pv-preview')).toBeNull()
})

it('renders castling and the rest of the engine line from an earlier game position', () => {
  const fen = 'r3k2r/ppp2ppp/2n1bn2/3qp3/3P4/2N1BN2/PPP2PPP/R2QK2R b KQkq - 5 8'
  const moves = pvSan(fen, ['e8g8', 'e1g1', 'e5d4'])
  expect(moves.map((move) => move.san)).toEqual(['O-O', 'O-O', 'exd4'])
  expect(moves[0]?.label).toBe('8… O-O')
  const wrapper = mount(EngineLines, {
    props: { fen, lines: [{ ...line, pv: ['e8g8', 'e1g1', 'e5d4'] }], orientation: 'white' },
    global: { stubs: { ChessBoard: true } },
  })
  expect(wrapper.findAll('.pv-move')).toHaveLength(3)
  wrapper.unmount()
  const tree = newTree(fen)
  expect(addMove(tree, '', 'e8g8')).toBe('e8h8')
  expect(addMove(tree, '', 'e8h8')).toBe('e8h8')
  expect(tree.children).toHaveLength(1)
})
