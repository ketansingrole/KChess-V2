import { mount, shallowMount } from '@vue/test-utils'
import { expect, it } from 'vitest'
import { nextTick } from 'vue'
import { INITIAL_FEN } from '@kchess/rules/position'
import EngineLines from '../../app/components/EngineLines.vue'
import AnalysisPage from '../../app/pages/analysis.vue'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop } from './fixtures'
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

// Stepping onto checkmate hid the lines, so everything below them moved up.
it('keeps the room of its rows when a game has ended', async () => {
  const wrapper = mount(EngineLines, {
    props: { fen: INITIAL_FEN, lines: [line], orientation: 'white', pending: 3, ended: 'Over.' },
  })
  const rows = wrapper.findAll('.pv-row')
  expect(rows).toHaveLength(1)
  expect(rows[0]!.classes()).toContain('ended')
  expect(rows[0]!.text()).toBe('Over.')
  expect(rows[0]!.attributes('style')).toContain('--rows: 3')
  wrapper.unmount()
})

it('shows the ended lines at checkmate on the analysis board', async () => {
  desktop({
    reviewGet: async () => null,
    onReviewUpdate: () => () => {},
    onReviewStatus: () => () => {},
    reviewStatus: async () => ({ waiting: 0 }),
    onAnalysis: () => () => {},
    startAnalysis: async () => 0,
    stopAnalysis: async () => {},
  })
  await useKChessStore().init()
  const analysis = useAnalysisStore()
  analysis.engineOn = true
  for (const uci of ['f2f3', 'e7e5', 'g2g4', 'd8h4']) analysis.play(uci)
  const stubs = Object.fromEntries(
    [
      'PageHeader',
      'StudyBar',
      'EvalBar',
      'ChessBoard',
      'AnalysisLine',
      'AnalysisReview',
      'MoveNotes',
    ].map((name) => [name, true]),
  )
  const page = shallowMount(AnalysisPage, { global: { stubs: { ...stubs, EngineLines: false } } })
  await nextTick()
  const ended = page.get('.pv-row.ended')
  expect(ended.text()).toBe('Nothing to analyse.')
  expect(ended.attributes('style')).toContain(`--rows: ${analysis.engineLines}`)
  analysis.back()
  await nextTick()
  expect(page.find('.pv-row.ended').exists()).toBe(false)
  expect(page.findAll('.pv-row')).toHaveLength(analysis.engineLines)
  page.unmount()
})
