import { expect, it, vi } from 'vitest'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { defineComponent } from 'vue'
import KnightPath from '../../app/components/KnightPath.vue'
import ChessBoard from '../../app/components/ChessBoard.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop, componentStubs, deferred } from './fixtures'
import type { RunInput, RunSummary } from '@kchess/contracts/types'
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

/** The squares of a shortest knight route, after the start square. */
function route(from: SquareName, goal: SquareName): SquareName[] {
  const queue: SquareName[][] = [[from]]
  const seen = new Set([from])
  for (let path = queue.shift(); path; path = queue.shift()) {
    const last = path.at(-1)!
    if (last === goal) return path.slice(1)
    for (const to of knightMoves(last) as SquareName[])
      if (!seen.has(to)) {
        seen.add(to)
        queue.push([...path, to])
      }
  }
  throw new Error(`No route from ${from} to ${goal}`)
}
const knightSquare = (fen: string) => ALL_SQUARES.find((s) => knightFen(s) === fen) as SquareName

// The saved scores load while Start is usable; a late load reset the running challenge to the
// idle board, "Reach e4 in 0 jumps".
it('keeps a running challenge when the saved scores load late', async () => {
  const summary = deferred<RunSummary>()
  desktop({ runSummary: () => summary.promise })
  await useKChessStore().init()
  const wrapper = mount(KnightPath, { global: { stubs } })
  await button(wrapper, 'Start').trigger('click')
  const target = wrapper.find('.side-panel .knight-target').text()
  const board = wrapper.findComponent(BoardStub)
  const fen = board.props('fen')
  summary.resolve({ kind: 'knightPath', best: {}, recent: [], total: 0 })
  await flushPromises()
  expect(wrapper.find('.side-panel .knight-target').text()).toBe(target)
  expect(target).not.toContain('in 0 jumps')
  expect(board.props('fen')).toBe(fen)
  wrapper.unmount()
})

// The 450 ms advance after a reached goal outlived Skip and restarts, and replaced a later round.
it('does not let the advance after a reached goal replace the next challenge', async () => {
  vi.useFakeTimers()
  desktop({
    saveRun: async (run: RunInput) => ({
      isBest: false,
      summary: { kind: run.kind, best: {}, recent: [], total: 1 },
    }),
  })
  await useKChessStore().init()
  const wrapper = mount(KnightPath, { global: { stubs } })
  await flushPromises()
  const board = wrapper.findComponent(BoardStub)
  const reachGoal = async () => {
    let from = knightSquare(board.props('fen'))
    for (const to of route(from, board.props('shapes')[0].orig)) {
      board.vm.$emit('move', `${from}${to}`)
      from = to
      await flushPromises()
    }
  }
  const jumpOnce = async () => {
    const from = knightSquare(board.props('fen'))
    const goal = board.props('shapes')[0].orig
    const to = (knightMoves(from) as SquareName[]).find((s) => s !== goal)!
    board.vm.$emit('move', `${from}${to}`)
    await flushPromises()
  }

  await button(wrapper, 'Start').trigger('click')
  await reachGoal()
  const reached = knightSquare(board.props('fen'))
  // The reached goal takes no more jumps while the next round is pending.
  board.vm.$emit('move', `${reached}${knightMoves(reached)[0]}`)
  await flushPromises()
  expect(knightSquare(board.props('fen'))).toBe(reached)
  await button(wrapper, 'Skip').trigger('click')
  await jumpOnce()
  const fen = board.props('fen')
  expect(wrapper.text()).toContain('1 so far')
  await vi.advanceTimersByTimeAsync(450)
  expect(board.props('fen')).toBe(fen)
  expect(wrapper.text()).toContain('1 so far')

  // Ending and restarting the run cancels it too.
  await reachGoal()
  await button(wrapper, 'End run').trigger('click')
  await flushPromises()
  await button(wrapper, 'Play again').trigger('click')
  await jumpOnce()
  const restarted = board.props('fen')
  await vi.advanceTimersByTimeAsync(450)
  expect(board.props('fen')).toBe(restarted)
  expect(wrapper.text()).toContain('1 so far')
  wrapper.unmount()
})
