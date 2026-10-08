import { describe, expect, it, vi } from 'vitest'
import { INITIAL_FEN } from 'chessops/fen'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { defineComponent } from 'vue'
import ChessBoard from '../../app/components/ChessBoard.vue'
import CoordinatesTrainer from '../../app/components/CoordinatesTrainer.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { EMPTY_FEN } from '../../src/shared/coordinates'
import { desktop, componentStubs } from './fixtures'
import type { RunInput } from '../../src/shared/types'

const BoardStub = defineComponent({
  name: 'ChessBoard',
  props: ['fen', 'orientation', 'coordinates', 'shapes'],
  emits: ['select'],
  template: '<div data-board />',
})
const TabsStub = defineComponent({
  name: 'UTabs',
  props: ['modelValue', 'disabled'],
  emits: ['update:modelValue'],
  template: '<div data-tabs />',
})
const SwitchStub = defineComponent({
  name: 'USwitch',
  props: ['modelValue', 'label', 'disabled'],
  emits: ['update:modelValue'],
  template:
    '<button role="switch" :aria-checked="modelValue" :disabled="disabled" @click="$emit(\'update:modelValue\', !modelValue)">{{ label }}</button>',
})
const stubs = {
  ...componentStubs,
  ChessBoard: BoardStub,
  UTabs: TabsStub,
  USwitch: SwitchStub,
  VoiceInput: true,
}

function button(wrapper: VueWrapper, text: string) {
  const found = wrapper.findAll('button').find((b) => b.text() === text)
  if (!found) throw new Error(`No button "${text}"`)
  return found
}

async function trainer(saveRun = vi.fn()) {
  saveRun.mockImplementation(async (run: RunInput) => ({
    isBest: true,
    summary: { kind: run.kind, best: {}, recent: [], total: 1 },
  }))
  desktop({ saveRun })
  await useKChessStore().init()
  const wrapper = mount(CoordinatesTrainer, { global: { stubs } })
  await flushPromises()
  const board = () => wrapper.findComponent(BoardStub)
  const [modeTabs, sideTabs] = wrapper.findAllComponents(TabsStub)
  return { wrapper, board, modeTabs: modeTabs!, sideTabs: sideTabs!, saveRun }
}

describe('coordinates trainer controls', () => {
  it('shows and hides coordinates on the board with the switch', async () => {
    const { wrapper, board } = await trainer()
    expect(board().props('coordinates')).toBe('none')
    await wrapper.find('[role="switch"]').trigger('click')
    expect(board().props('coordinates')).not.toBe('none')
    await wrapper.find('[role="switch"]').trigger('click')
    expect(board().props('coordinates')).toBe('none')
    wrapper.unmount()
  })

  it('turns the board to the chosen side before a run starts', async () => {
    const { wrapper, board, sideTabs } = await trainer()
    // The pieces are what make the chosen side visible; an empty board looks the same from both.
    expect(board().props('fen')).toBe(INITIAL_FEN)
    expect(board().props('orientation')).toBe('white')
    sideTabs.vm.$emit('update:modelValue', 'black')
    await flushPromises()
    expect(board().props('orientation')).toBe('black')
    sideTabs.vm.$emit('update:modelValue', 'white')
    await flushPromises()
    expect(board().props('orientation')).toBe('white')
    wrapper.unmount()
  })

  it('scores clicks in find mode, locks settings while running and saves the run', async () => {
    const { wrapper, board, modeTabs, sideTabs, saveRun } = await trainer()
    await button(wrapper, 'Start').trigger('click')
    expect(wrapper.find('.run-hud').exists()).toBe(true)
    expect(board().props('fen')).toBe(EMPTY_FEN)
    expect(modeTabs.props('disabled')).toBe(true)
    expect(sideTabs.props('disabled')).toBe(true)
    expect(wrapper.find('[role="switch"]').attributes('disabled')).toBeDefined()
    const target = wrapper.find('.coord-target').text()
    board().vm.$emit('select', target === 'h8' ? 'a1' : 'h8')
    board().vm.$emit('select', target)
    await flushPromises()
    expect(wrapper.find('.run-score strong').text()).toBe('1')
    await button(wrapper, 'End run').trigger('click')
    await flushPromises()
    expect(saveRun).toHaveBeenCalledWith(
      expect.objectContaining({
        variant: 'find',
        score: 1,
        detail: expect.objectContaining({ mistakes: 1, accuracy: 50 }),
      }),
    )
    expect(wrapper.text()).toContain('1 correct · 50% accuracy')
    expect(button(wrapper, 'Play again').exists()).toBe(true)
    wrapper.unmount()
  })

  it('answers with the file and rank buttons and the keyboard in name mode', async () => {
    const { wrapper, board, modeTabs } = await trainer()
    modeTabs.vm.$emit('update:modelValue', 'name')
    await flushPromises()
    await button(wrapper, 'Start').trigger('click')
    const target = () => (board().props('shapes') as { orig: string }[])[0]!.orig
    const first = target()
    await button(wrapper, first[0]!).trigger('click')
    expect(wrapper.find('.coord-target').text()).toBe(`${first[0]}?`)
    await button(wrapper, first[1]!).trigger('click')
    expect(wrapper.find('.run-score strong').text()).toBe('1')
    const second = target()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: second[0]!.toUpperCase() }))
    window.dispatchEvent(new KeyboardEvent('keydown', { key: second[1]! }))
    await flushPromises()
    expect(wrapper.find('.run-score strong').text()).toBe('2')
    wrapper.unmount()
  })
})

describe('chess board coordinates', () => {
  it('draws and removes coordinate labels when the setting changes on a mounted board', async () => {
    const wrapper = mount(ChessBoard, {
      props: { fen: EMPTY_FEN, coordinates: 'none' },
      attachTo: document.body,
    })
    await flushPromises()
    expect(wrapper.find('coords').exists()).toBe(false)
    await wrapper.setProps({ coordinates: 'inside' })
    expect(wrapper.findAll('coords').length).toBeGreaterThan(0)
    await wrapper.setProps({ coordinates: 'none' })
    expect(wrapper.find('coords').exists()).toBe(false)
    wrapper.unmount()
  })
})
