import { expect, it, vi } from 'vitest'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { defineComponent } from 'vue'
import KnightPath from '../../app/components/KnightPath.vue'
import ChessBoard from '../../app/components/ChessBoard.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop, componentStubs } from './fixtures'
import type { RunInput } from '@kchess/contracts/types'
import { ALL_SQUARES } from '@kchess/rules/coordinates'
import { knightFen, knightMoves } from '@kchess/rules/knight'
import type { SquareName } from '@kchess/rules/position'

const RANKS = ['one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight']

const BoardStub = defineComponent({
  name: 'ChessBoard',
  props: ['fen', 'shapes', 'movable'],
  emits: ['move'],
  template: '<div data-board />',
})
const TabsStub = defineComponent({
  name: 'UTabs',
  props: ['modelValue', 'disabled'],
  emits: ['update:modelValue'],
  template: '<div data-tabs />',
})
const stubs = { ...componentStubs, ChessBoard: BoardStub, UTabs: TabsStub, USelect: true }

function button(wrapper: VueWrapper, text: string) {
  const found = wrapper.findAll('button').find((b) => b.text() === text)
  if (!found) throw new Error(`No button "${text}"`)
  return found
}

/*
 * Knight paths was always a 60-second run, announced in a row above the board that took the
 * board's height. Runs now take any length or none, and the target sits in the side panel.
 */
it('keeps a best per length, none for untimed practice, and the target beside the board', async () => {
  const saveRun = vi.fn(async (run: RunInput) => ({
    isBest: false,
    summary: { kind: run.kind, best: {}, recent: [], total: 1 },
  }))
  desktop({ saveRun })
  await useKChessStore().init()
  const wrapper = mount(KnightPath, { global: { stubs } })
  await flushPromises()
  const length = wrapper.findComponent(TabsStub)
  expect(wrapper.text()).not.toContain('Press Start')

  await button(wrapper, 'Start').trigger('click')
  expect(length.props('disabled')).toBe(true)
  expect(wrapper.find('.side-panel .knight-target').text()).toMatch(/^Reach [a-h][1-8] in \d/)
  await button(wrapper, 'End run').trigger('click')
  await flushPromises()
  expect(saveRun).toHaveBeenLastCalledWith(expect.objectContaining({ variant: 'medium' }))

  length.vm.$emit('update:modelValue', 120)
  await flushPromises()
  await button(wrapper, 'Play again').trigger('click')
  await button(wrapper, 'End run').trigger('click')
  await flushPromises()
  expect(saveRun).toHaveBeenLastCalledWith(expect.objectContaining({ variant: 'medium-120s' }))

  saveRun.mockClear()
  length.vm.$emit('update:modelValue', 0)
  await flushPromises()
  await button(wrapper, 'Play again').trigger('click')
  await button(wrapper, 'End run').trigger('click')
  await flushPromises()
  expect(saveRun).not.toHaveBeenCalled()
  wrapper.unmount()
})

// Knight paths had no voice input. A spoken square is the jump; one the knight cannot reach is
// refused, and both land in the voice history.
it('jumps to a spoken square and refuses one the knight cannot reach', async () => {
  const prepare = vi.fn(async () => true)
  const VoiceStub = defineComponent({
    name: 'VoiceInput',
    props: ['enabled', 'active', 'feedback'],
    emits: ['update:enabled', 'result', 'unclear'],
    setup: () => ({ prepare }),
    template: '<div data-voice>{{ feedback }}</div>',
  })
  const saveVoiceAttempt = vi.fn(async () => 1)
  desktop({
    saveVoiceAttempt,
    saveRun: vi.fn(async (run: RunInput) => ({
      isBest: false,
      summary: { kind: run.kind, best: {}, recent: [], total: 1 },
    })),
  })
  await useKChessStore().init()
  const wrapper = mount(KnightPath, {
    global: { stubs: { ...stubs, VoiceInput: VoiceStub } },
  })
  await flushPromises()
  const voice = wrapper.findComponent(VoiceStub)
  const board = () => wrapper.findComponent(BoardStub)
  voice.vm.$emit('update:enabled', true)
  await button(wrapper, 'Start').trigger('click')
  await flushPromises()
  expect(prepare).toHaveBeenCalled()

  const knightOn = () => ALL_SQUARES.find((square) => knightFen(square) === board().props('fen'))!
  const spoken = (square: string) => `${square[0]} ${RANKS[Number(square[1]) - 1]}`
  const said = { confidence: 0.9, words: [] }
  const from = knightOn()
  const to = knightMoves(from)[0]!
  voice.vm.$emit('result', { ...said, text: spoken(to) })
  await flushPromises()
  expect(knightOn()).toBe(to)
  expect(saveVoiceAttempt).toHaveBeenLastCalledWith(
    expect.objectContaining({ source: 'knightPath', parsed: to, outcome: 'played' }),
  )

  const unreachable = ALL_SQUARES.find(
    (square) => square !== knightOn() && !knightMoves(knightOn()).includes(square),
  )!
  voice.vm.$emit('result', { ...said, text: spoken(unreachable) })
  await flushPromises()
  expect(knightOn()).toBe(to)
  expect(wrapper.find('[data-voice]').text()).toBe(`${unreachable} · not a knight jump from ${to}`)
  expect(saveVoiceAttempt).toHaveBeenLastCalledWith(
    expect.objectContaining({ source: 'knightPath', parsed: unreachable, outcome: 'invalid' }),
  )
  wrapper.unmount()
})

// The typed-move box wanted a full move (Nf3, g1f3), which a lone knight with no kings can't
// make; a drill board now takes the square alone. Games still need a full move.
it('takes a typed square as the move on a drill board, and blindfolds the knight', async () => {
  const from = 'e4'
  const reach = knightMoves(from)
  const dests = new Map<SquareName, SquareName[]>([[from, reach as SquareName[]]])
  const board = mount(ChessBoard, {
    props: {
      fen: knightFen(from),
      dests,
      movable: true,
      interactive: true,
      movableColor: 'white',
      turnColor: 'white',
      squareInput: true,
      blindfold: true,
    },
    attachTo: document.body,
  })
  await flushPromises()
  expect(board.find('.cg-host').classes()).toContain('blindfold')
  const input = board.find<HTMLInputElement>('input.board-move-input')
  expect(input.attributes('placeholder')).toBe('Type a square: f3')
  await input.setValue('h8')
  await board.find('form.board-move-form').trigger('submit')
  expect(board.emitted('move')).toBeUndefined()
  await input.setValue(`N${reach[0]}`)
  await board.find('form.board-move-form').trigger('submit')
  expect(board.emitted('move')).toEqual([[`${from}${reach[0]}`]])
  board.unmount()
})
