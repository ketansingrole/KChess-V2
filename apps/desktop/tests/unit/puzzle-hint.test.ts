import { expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import PuzzleBoard from '../../app/components/PuzzleBoard.vue'
import PuzzleStatusPanel from '../../app/components/PuzzleStatusPanel.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop, puzzle, componentStubs, ChessBoardStub } from './fixtures'

it('toggles the hint: one press shows the piece to move, the next hides it', async () => {
  desktop()
  await useKChessStore().init()
  const board = mount(PuzzleBoard, { props: { puzzle }, global: { stubs: componentStubs } })
  const shapes = () => board.findComponent(ChessBoardStub).vm.$attrs.shapes
  expect(shapes()).toEqual([])
  board.vm.hint()
  await board.vm.$nextTick()
  expect(board.vm.hinted).toBe(true)
  expect(shapes()).toEqual([
    { orig: 'a1', below: true, customSvg: { html: expect.stringContaining('cg-hint') } },
  ])
  board.vm.hint()
  await board.vm.$nextTick()
  expect(board.vm.hinted).toBe(false)
  expect(shapes()).toEqual([])
  board.unmount()
})

it('shows the hint button as pressed while the hint is on', () => {
  const panel = (hinted: boolean) =>
    mount(PuzzleStatusPanel, {
      props: {
        puzzle,
        status: 'playing',
        outcome: null,
        feedback: '',
        waiting: false,
        mistakes: 0,
        hinted,
      },
      global: { stubs: { ...componentStubs, UIcon: true } },
    })
  const hint = (hinted: boolean) =>
    panel(hinted)
      .findAll('button')
      .find((button) => button.text() === 'Hint')!
  expect(hint(false).attributes('aria-pressed')).toBe('false')
  expect(hint(true).attributes('aria-pressed')).toBe('true')
})
