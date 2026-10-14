import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent } from 'vue'
import PuzzleTrain from '../../app/components/PuzzleTrain.vue'
import PuzzleBoard from '../../app/components/PuzzleBoard.vue'
import PuzzleVoice from '../../app/components/PuzzleVoice.vue'
import VoiceMoveInput from '../../app/components/VoiceMoveInput.vue'
import MaterialDiff from '../../app/components/MaterialDiff.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { usePuzzleStore } from '../../app/stores/puzzles'
import { DEFAULT_SETTINGS } from '@kchess/contracts/defaultSettings'
import { desktop, puzzle, componentStubs, ChessBoardStub } from './fixtures'

const StatusStub = defineComponent({
  name: 'PuzzleStatusPanel',
  emits: ['hint', 'solution'],
  template: `<div><button data-solution @click="$emit('solution')">Solution</button><slot name="actions" /></div>`,
})
const global = {
  components: { PuzzleBoard, PuzzleVoice, VoiceMoveInput, MaterialDiff },
  stubs: {
    ...componentStubs,
    PuzzleStatusPanel: StatusStub,
    VoiceInput: true,
    UTabs: true,
    UEmpty: true,
    UTooltip: { template: '<div><slot /></div>' },
  },
}

async function train(autoNext: boolean) {
  let draws = 0
  const puzzleNext = vi.fn(async () => ({ puzzle: { ...puzzle, id: `p${++draws}` } }))
  desktop({ puzzleNext: puzzleNext as never })
  const app = useKChessStore()
  await app.init()
  app.settings!.puzzleAutoNext = autoNext
  usePuzzleStore().mode = 'practice'
  const wrapper = mount(PuzzleTrain, { global })
  await flushPromises()
  return { wrapper, puzzleNext, puzzles: usePuzzleStore() }
}

afterEach(() => {
  vi.useRealTimers()
})

describe('next puzzle after solving', () => {
  it('is on by default', () => {
    expect(DEFAULT_SETTINGS.puzzleAutoNext).toBe(true)
  })

  it('moves on after a short pause once the puzzle is solved', async () => {
    const { wrapper, puzzleNext, puzzles } = await train(true)
    expect(puzzles.current?.id).toBe('p1')
    vi.useFakeTimers()
    wrapper.findComponent(ChessBoardStub).vm.$emit('move', 'a1a8')
    await flushPromises()
    expect(puzzleNext).toHaveBeenCalledOnce()
    await vi.advanceTimersByTimeAsync(800)
    await flushPromises()
    expect(puzzleNext).toHaveBeenCalledTimes(2)
    expect(puzzles.current?.id).toBe('p2')
    wrapper.unmount()
  })

  it('stays on the solved puzzle when the setting is off', async () => {
    const { wrapper, puzzleNext } = await train(false)
    vi.useFakeTimers()
    wrapper.findComponent(ChessBoardStub).vm.$emit('move', 'a1a8')
    await vi.advanceTimersByTimeAsync(5_000)
    expect(puzzleNext).toHaveBeenCalledOnce()
    wrapper.unmount()
  })

  it('stays on a puzzle whose solution was shown', async () => {
    const { wrapper, puzzleNext } = await train(true)
    vi.useFakeTimers()
    await wrapper.get('[data-solution]').trigger('click')
    await vi.advanceTimersByTimeAsync(5_000)
    expect(wrapper.findComponent(PuzzleBoard).vm.status).toBe('revealed')
    expect(puzzleNext).toHaveBeenCalledOnce()
    wrapper.unmount()
  })
})
