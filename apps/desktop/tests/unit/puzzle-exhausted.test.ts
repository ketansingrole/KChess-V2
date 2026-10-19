import { describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent, type PropType } from 'vue'
import PuzzleTrain from '../../app/components/PuzzleTrain.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { usePuzzleStore } from '../../app/stores/puzzles'
import { DEFAULT_SETTINGS } from '@kchess/contracts/defaultSettings'
import { desktop, puzzle, componentStubs } from './fixtures'

const EmptyStub = defineComponent({
  name: 'UEmpty',
  props: {
    title: { type: String, default: '' },
    description: { type: String, default: '' },
    actions: {
      type: Array as PropType<{ label: string; onClick(): void }[]>,
      default: () => [],
    },
  },
  template: `<div data-empty><h2>{{ title }}</h2><p>{{ description }}</p>
    <button v-for="action in actions" :key="action.label" @click="action.onClick()">{{ action.label }}</button></div>`,
})
const Blank = defineComponent({ inheritAttrs: false, template: '<div />' })
const global = {
  directives: { boardFit: {} },
  stubs: {
    ...componentStubs,
    UEmpty: EmptyStub,
    PuzzleBoard: Blank,
    PuzzleVoice: Blank,
    PuzzleStatusPanel: Blank,
    UTabs: true,
  },
}

/** Puzzles → Train, practising an opening at Harder, where Lichess has nothing left. */
async function exhausted() {
  const puzzleNext = vi.fn(async (request: { difficulty: string; angle: string }) =>
    request.difficulty === 'harder' && request.angle !== 'mix'
      ? { glicko: { rating: 1620 } }
      : { puzzle },
  )
  desktop({
    puzzleNext: puzzleNext as never,
    loadData: async () => ({
      settings: { ...DEFAULT_SETTINGS },
      accounts: [{ username: 'Alice', connected: true }],
      gameCount: 0,
    }),
  })
  await useKChessStore().init()
  const puzzles = usePuzzleStore()
  puzzles.mode = 'practice'
  puzzles.angle = 'Anderssens_Opening_Polish_Gambit'
  puzzles.difficulty = 'harder'
  const wrapper = mount(PuzzleTrain, { global })
  await flushPromises()
  return { wrapper, puzzles, puzzleNext }
}

const buttons = (wrapper: Awaited<ReturnType<typeof exhausted>>['wrapper']) =>
  wrapper.find('[data-empty]').findAll('button')

describe('a theme with no puzzles left', () => {
  it('is a calm choice, not an error, and an easier difficulty draws again', async () => {
    const { wrapper, puzzles, puzzleNext } = await exhausted()
    expect(puzzleNext).toHaveBeenLastCalledWith(
      expect.objectContaining({ account: 'Alice', difficulty: 'harder' }),
    )
    expect(puzzles.phase).toBe('exhausted')
    expect(puzzles.trainError).toBe('')
    const text = wrapper.text()
    expect(text).toContain('No more Harder puzzles for this theme')
    expect(text).not.toContain('Couldn’t get a puzzle')
    expect(text).not.toContain('Try again')
    expect(text).not.toContain('Error')
    expect(buttons(wrapper).map((button) => button.text())).toEqual([
      'Try Normal',
      'Try Healthy mix',
      'Use downloaded puzzles',
    ])

    await buttons(wrapper)[0]!.trigger('click')
    await flushPromises()
    expect(puzzles.difficulty).toBe('normal')
    expect(puzzleNext).toHaveBeenLastCalledWith(expect.objectContaining({ difficulty: 'normal' }))
    expect(puzzles.phase).toBe('ready')
    expect(puzzles.current?.id).toBe(puzzle.id)
  })

  it('offers the mixed themes and downloaded puzzles too', async () => {
    const { wrapper, puzzles, puzzleNext } = await exhausted()
    await buttons(wrapper)[1]!.trigger('click')
    await flushPromises()
    expect(puzzles.angle).toBe('mix')
    expect(puzzleNext).toHaveBeenLastCalledWith(expect.objectContaining({ angle: 'mix' }))
    expect(puzzles.phase).toBe('ready')
  })
})
