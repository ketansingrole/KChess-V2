import { describe, expect, it } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent, nextTick, ref } from 'vue'
import PuzzleBoard from '../../app/components/PuzzleBoard.vue'
import PuzzleDaily from '../../app/components/PuzzleDaily.vue'
import MaterialDiff from '../../app/components/MaterialDiff.vue'
import VoiceMoveInput from '../../app/components/VoiceMoveInput.vue'
import PuzzleVoice from '../../app/components/PuzzleVoice.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop, puzzle, componentStubs } from './fixtures'

const VoiceStub = defineComponent({
  name: 'VoiceInput',
  props: ['active', 'contextKey', 'grammar', 'enabled', 'feedback'],
  emits: ['update:enabled', 'result', 'unclear'],
  template: '<div data-voice :data-active="active"><slot /></div>',
})
const global = {
  components: { PuzzleBoard, MaterialDiff, VoiceMoveInput, PuzzleVoice },
  stubs: { ...componentStubs, VoiceInput: VoiceStub, PuzzleStatusPanel: true },
}

async function daily() {
  desktop({ puzzleDaily: async () => puzzle })
  await useKChessStore().init()
  const wrapper = mount(PuzzleDaily, { global })
  await flushPromises()
  const voice = wrapper.findComponent(VoiceStub)
  voice.vm.$emit('update:enabled', true)
  await flushPromises()
  const board = wrapper.findComponent(PuzzleBoard)
  const say = async (text: string) => {
    voice.vm.$emit('result', { text, confidence: 0.95 })
    await flushPromises()
  }
  return { wrapper, voice, board, say }
}

describe('puzzle board extras', () => {
  it('shows the material lead on the solver’s side and nothing for the side behind', async () => {
    const { wrapper } = await daily()
    // White is up the a1 rook: the lead sits below the board, where White plays from.
    expect(wrapper.get('[data-side="bottom"] .material-lead').text()).toBe('+5')
    expect(wrapper.find('[data-side="top"] .material').exists()).toBe(false)
    wrapper.unmount()
  })

  it('leaves timed boards (no `material`) without material rows', async () => {
    desktop()
    await useKChessStore().init()
    const wrapper = mount(PuzzleBoard, { props: { puzzle }, global })
    expect(wrapper.find('.puzzle-material').exists()).toBe(false)
    wrapper.unmount()
  })

  it('starts a board mounted without a retained verdict as a fresh attempt', async () => {
    desktop()
    await useKChessStore().init()
    const outcomes: boolean[] = []
    const wrapper = mount(PuzzleBoard, {
      props: { puzzle, onOutcome: (win: boolean) => outcomes.push(win) },
      global,
    })
    expect(wrapper.vm.outcome).toBeNull()
    expect(wrapper.vm.mistakes).toBe(0)
    wrapper.findComponent({ name: 'ChessBoard' }).vm.$emit('move', 'a1a8')
    await flushPromises()
    expect(outcomes).toEqual([true])
    wrapper.unmount()
  })

  it('solves a puzzle by voice once the spoken move is confirmed', async () => {
    const { wrapper, voice, board, say } = await daily()
    expect(voice.props('active')).toBe(true)
    await say('rook a eight')
    expect(board.vm.status).toBe('playing')
    expect(wrapper.text()).toContain('Ra8')
    await say('confirm')
    expect(board.vm.status).toBe('solved')
    expect(board.vm.outcome).toBe(true)
    expect(voice.props('active')).toBe(false)
    wrapper.unmount()
  })

  it('counts a spoken wrong move as a mistake, like one made on the board', async () => {
    const { wrapper, board, say } = await daily()
    await say('rook a seven')
    await say('confirm')
    expect(board.vm.mistakes).toBe(1)
    expect(board.vm.outcome).toBe(false)
    expect(board.vm.status).toBe('playing')
    wrapper.unmount()
  })

  it('puts voice in the bottom row of each new board without restarting it', async () => {
    desktop()
    await useKChessStore().init()
    const second = { ...puzzle, id: 'mate456' }
    const Page = defineComponent({
      components: { PuzzleBoard, PuzzleVoice },
      setup() {
        const current = ref(puzzle)
        const board = ref<InstanceType<typeof PuzzleBoard> | null>(null)
        return { current, board }
      },
      template: `<div>
        <PuzzleBoard ref="board" :key="current.id" :puzzle="current" material />
        <PuzzleVoice :board="board" :context-key="current.id" />
      </div>`,
    })
    const wrapper = mount(Page, { attachTo: document.body, global })
    await flushPromises()
    const voice = wrapper.findComponent(VoiceStub)
    const rowOf = () => voice.element.closest('[data-side="bottom"]')
    expect(rowOf()).not.toBeNull()
    const firstRow = rowOf()
    wrapper.vm.current = second
    await nextTick()
    await flushPromises()
    expect(wrapper.findComponent(VoiceStub).vm).toBe(voice.vm)
    expect(rowOf()).not.toBeNull()
    expect(rowOf()).not.toBe(firstRow)
    wrapper.unmount()
  })
})
