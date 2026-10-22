import { expect, it, vi } from 'vitest'
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

it('takes no moves while the solution plays itself out', async () => {
  // A move during the replay advanced the solution and played the opponent's reply as the player's.
  vi.useFakeTimers()
  desktop()
  await useKChessStore().init()
  const twoMoves = {
    ...puzzle,
    fen: '6k1/5ppp/8/8/8/8/5PPP/RR4K1 w - - 0 1',
    solution: ['b1b8', 'g8h8', 'a1a8'],
  }
  const board = mount(PuzzleBoard, {
    props: { puzzle: twoMoves },
    global: { stubs: componentStubs },
  })
  const replay = board.vm.showSolution()
  await board.vm.$nextTick()
  expect(board.vm.playing).toBe(false)
  expect(board.vm.playMove('b1b8')).toBe(false)
  await vi.advanceTimersByTimeAsync(3000)
  await replay
  expect(board.vm.status).toBe('revealed')
  board.unmount()
  vi.useRealTimers()
})

it('asks for the promotion piece when queening applies only to premoves', async () => {
  // "Queen when premoving" queened every puzzle promotion, so an underpromotion could not be found.
  desktop()
  const kchess = useKChessStore()
  await kchess.init()
  kchess.settings.promotion = 'premove'
  const board = mount(PuzzleBoard, { props: { puzzle }, global: { stubs: componentStubs } })
  expect(board.findComponent(ChessBoardStub).props('promotion')).toBe('ask')
  kchess.settings.promotion = 'queen'
  await board.vm.$nextTick()
  expect(board.findComponent(ChessBoardStub).props('promotion')).toBe('queen')
  board.unmount()
})
