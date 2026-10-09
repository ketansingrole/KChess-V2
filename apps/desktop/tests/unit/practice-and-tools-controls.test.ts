import { describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent, nextTick } from 'vue'
import Editor from '../../app/pages/editor.vue'
import SquareColorTrainer from '../../app/components/SquareColorTrainer.vue'
import { useLocalGameStore } from '../../app/stores/local'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useKChessStore } from '../../app/stores/kchess'
import { squareColor, type Square } from '@kchess/core/domain/coordinates'
import { desktop, componentStubs } from './fixtures'
import type { RunInput } from '@kchess/core/contracts/types'

const InputStub = defineComponent({
  name: 'UInput',
  inheritAttrs: false,
  props: ['modelValue'],
  emits: ['update:modelValue'],
  template:
    '<input v-bind="$attrs" :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
})

describe('over-the-board chess clock', () => {
  it('shows typed minutes on a clock nobody has started', async () => {
    const board = useLocalGameStore()
    board.otbConfig.minutes = 3
    board.otbConfig.bottomMinutes = 10
    await nextTick()
    expect([board.otbLeft('top'), board.otbLeft('bottom')]).toEqual([180_000, 600_000])
    // Retyping clears the field for a moment; the clock follows once a number is back.
    board.otbConfig.minutes = '' as unknown as number
    await nextTick()
    board.otbConfig.minutes = 15
    await nextTick()
    expect(board.otbLeft('top')).toBe(900_000)
  })

  it('keeps the time of a clock that has started when the minutes change', async () => {
    const board = useLocalGameStore()
    board.otbPress('bottom')
    board.otbPause()
    const top = board.otbLeft('top')
    board.otbConfig.minutes = 1
    await nextTick()
    expect(board.otbLeft('top')).toBe(top)
    board.otbReset()
    expect(board.otbLeft('top')).toBe(60_000)
    board.otbConfig.minutes = 2
    await nextTick()
    expect(board.otbLeft('top')).toBe(120_000)
  })
})

describe('board editor FEN box', () => {
  it('loads a typed FEN when Enter is pressed', async () => {
    desktop()
    await useKChessStore().init()
    const wrapper = mount(Editor, {
      global: {
        stubs: {
          ...componentStubs,
          UInput: InputStub,
          EditorBoard: true,
          VoiceInput: true,
          PageHeader: true,
          UTabs: true,
          UTooltip: { template: '<div><slot /></div>' },
        },
      },
    })
    const input = wrapper.find('#editor-fen')
    await input.setValue('4k3/8/8/8/8/8/8/4K2R w K - 0 1')
    await input.trigger('keydown', { key: 'Enter' })
    expect(useAnalysisStore().editor.board).toBe('4k3/8/8/8/8/8/8/4K2R')
    wrapper.unmount()
  })
})

describe('square colour trainer', () => {
  it('scores the light and dark buttons and the L and D keys', async () => {
    const saveRun = vi.fn(async (run: RunInput) => ({
      isBest: false,
      summary: { kind: run.kind, best: {}, recent: [], total: 1 },
    }))
    desktop({ saveRun })
    const wrapper = mount(SquareColorTrainer, { global: { stubs: componentStubs } })
    await flushPromises()
    const button = (text: string) => wrapper.findAll('button').find((b) => b.text().includes(text))!
    const shown = () => wrapper.find('.drill-square').text() as Square
    await button('Start').trigger('click')
    // One right answer with the buttons, one wrong one with the keyboard.
    await button(squareColor(shown()) === 'light' ? 'Light' : 'Dark').trigger('click')
    const wrongKey = squareColor(shown()) === 'light' ? 'd' : 'l'
    window.dispatchEvent(new KeyboardEvent('keydown', { key: wrongKey }))
    await flushPromises()
    expect(wrapper.text()).toContain('1 correct')
    wrapper.unmount()
  })
})

describe('over-the-board board turning', () => {
  it('turns the board to the side to move after each move while switched on', async () => {
    const board = useLocalGameStore()
    board.autoFlip = true
    board.move('e2e4')
    expect(board.orientation).toBe('black')
    board.move('e7e5')
    expect(board.orientation).toBe('white')
  })

  it('keeps the board where it is when switched off, and the flip button always flips', async () => {
    const board = useLocalGameStore()
    board.autoFlip = true
    board.move('e2e4')
    board.autoFlip = false
    await nextTick()
    expect(board.orientation).toBe('black')
    board.autoFlip = true
    await nextTick()
    board.flip()
    await nextTick()
    expect(board.autoFlip).toBe(false)
    expect(board.orientation).toBe('white')
    board.flip()
    expect(board.orientation).toBe('black')
  })
})
