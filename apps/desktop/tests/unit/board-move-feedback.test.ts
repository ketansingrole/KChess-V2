import { expect, it } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import ChessBoard from '../../app/components/ChessBoard.vue'
import BoardMoveFeedback from '../../app/components/BoardMoveFeedback.vue'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useReviewStore } from '../../app/stores/review'
import { useKChessStore } from '../../app/stores/kchess'
import { reviewKey } from '@kchess/rules/review'
import { desktop } from './fixtures'
import { INITIAL_FEN } from '@kchess/rules/position'

it('places feedback on the played square, follows flips and clears with navigation', async () => {
  const board = mount(ChessBoard, {
    props: { fen: INITIAL_FEN, moveFeedback: { square: 'f6', classification: 'blunder' } },
  })
  const square = () => board.get('.classified-square')
  expect(square().attributes('style')).toContain('left: 62.5%')
  expect(square().attributes('style')).toContain('top: 25%')
  expect(board.get('[role="img"]').attributes('aria-label')).toBe('Blunder move on f6')
  expect(board.get('.move-badge').text()).toBe('??')
  await board.setProps({ orientation: 'black' })
  expect(square().attributes('style')).toContain('left: 25%')
  expect(square().attributes('style')).toContain('top: 62.5%')
  await board.setProps({ moveFeedback: { square: 'h7', classification: 'brilliant' } })
  expect(board.get('.move-badge').text()).toBe('!!')
  await board.setProps({ moveFeedback: undefined })
  expect(board.findComponent(BoardMoveFeedback).exists()).toBe(false)
  board.unmount()
})

it('limits classifications to the reviewed main line and hides them during live play', async () => {
  desktop({ reviewGet: async () => null, onReviewUpdate: () => () => {} })
  const app = useKChessStore()
  await app.init()
  const analysis = useAnalysisStore()
  analysis.engineOn = false
  analysis.play('e2e4')
  const key = reviewKey(INITIAL_FEN, ['e2e4'])
  useReviewStore().reviews.set(key, {
    key,
    fen: INITIAL_FEN,
    moves: ['e2e4'],
    source: 'local',
    evals: [
      { cp: 20, best: 'e2e4', depth: 18 },
      { cp: 20, depth: 18 },
    ],
    depth: 18,
    complete: true,
    updatedAt: 0,
  })
  await flushPromises()
  expect(analysis.reviewMarks.get('e2e4')?.classification).toBe('best')
  analysis.back()
  analysis.play('d2d4')
  expect(analysis.reviewMarks.get('d2d4')).toBeUndefined()
  for (const phase of ['playing', 'seeking', 'disconnected'] as const) {
    app.onlinePhase = phase
    expect(analysis.reviewMarks.size).toBe(0)
  }
})
