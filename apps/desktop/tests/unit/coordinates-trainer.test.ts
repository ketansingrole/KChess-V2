import { describe, expect, it, vi } from 'vitest'
import { INITIAL_FEN } from '@kchess/rules/position'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { defineComponent } from 'vue'
import ChessBoard from '../../app/components/ChessBoard.vue'
import CoordinatesTrainer from '../../app/components/CoordinatesTrainer.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { loadViewPreferences } from '../../app/composables/useViewPreference'
import { EMPTY_FEN } from '@kchess/rules/coordinates'
import { desktop, componentStubs, deferred, keydownListeners } from './fixtures'
import type { RunInput, RunSummary } from '@kchess/contracts/types'

const BoardStub = defineComponent({
  name: 'ChessBoard',
  props: ['fen', 'orientation', 'coordinates', 'coordinatesOnSquares', 'shapes'],
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
  const [modeTabs, lengthTabs, sideTabs] = wrapper.findAllComponents(TabsStub)
  return {
    wrapper,
    board,
    modeTabs: modeTabs!,
    lengthTabs: lengthTabs!,
    sideTabs: sideTabs!,
    saveRun,
  }
}

describe('coordinates trainer controls', () => {
  it('shows and hides coordinates on the board with the switch', async () => {
    const { wrapper, board } = await trainer()
    expect(board().props('coordinates')).toBe('none')
    await button(wrapper, 'Show coordinates').trigger('click')
    expect(board().props('coordinates')).not.toBe('none')
    await button(wrapper, 'Show coordinates').trigger('click')
    expect(board().props('coordinates')).toBe('none')
    // Labels on every square need the board's coordinates on as well.
    await button(wrapper, 'Coordinates on every square').trigger('click')
    expect(board().props('coordinatesOnSquares')).toBe(true)
    expect(board().props('coordinates')).not.toBe('none')
    wrapper.unmount()
  })

  it('keeps the pieces on the board during a run when asked', async () => {
    const { wrapper, board } = await trainer()
    await button(wrapper, 'Show pieces').trigger('click')
    await button(wrapper, 'Start').trigger('click')
    expect(board().props('fen')).toBe(INITIAL_FEN)
    wrapper.unmount()
  })

  it('asks only the chosen files and ranks, keeps one of each, and saves no record', async () => {
    const { wrapper, board, saveRun } = await trainer()
    await button(wrapper, 'Practice only some files & ranks').trigger('click')
    for (const file of 'bcdefgh') await button(wrapper, file).trigger('click')
    for (const rank of '345678') await button(wrapper, rank).trigger('click')
    // The last file cannot be turned off.
    await button(wrapper, 'a').trigger('click')
    expect(button(wrapper, 'a').attributes('aria-pressed')).toBe('true')
    await button(wrapper, 'Start').trigger('click')
    for (let i = 0; i < 6; i++) {
      const target = wrapper.find('.coord-target').text()
      expect(['a1', 'a2']).toContain(target)
      board().vm.$emit('select', target)
      await flushPromises()
    }
    await button(wrapper, 'End run').trigger('click')
    await flushPromises()
    expect(saveRun).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('Not saved: some files & ranks only')
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

  // Runs were always 30 seconds. Longer runs keep their own best; untimed practice keeps none.
  it('keeps a best per run length and none for untimed practice', async () => {
    const { wrapper, board, lengthTabs, saveRun } = await trainer()
    lengthTabs.vm.$emit('update:modelValue', 120)
    await flushPromises()
    expect(wrapper.text()).toContain('Best (find, 2 min)')
    await button(wrapper, 'Start').trigger('click')
    expect(lengthTabs.props('disabled')).toBe(true)
    board().vm.$emit('select', wrapper.find('.coord-target').text())
    await button(wrapper, 'End run').trigger('click')
    await flushPromises()
    expect(saveRun).toHaveBeenCalledWith(
      expect.objectContaining({ variant: 'find-120s', score: 1 }),
    )

    saveRun.mockClear()
    lengthTabs.vm.$emit('update:modelValue', 0)
    await flushPromises()
    await button(wrapper, 'Play again').trigger('click')
    board().vm.$emit('select', wrapper.find('.coord-target').text())
    await button(wrapper, 'End run').trigger('click')
    await flushPromises()
    expect(saveRun).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('1 correct · 100% accuracy')
    lengthTabs.vm.$emit('update:modelValue', 30)
    wrapper.unmount()
  })

  // Audit 60a61e8 #4: another window's length choice, adopted mid-run, filed a 30-second run's
  // score under the 5-minute record; its file & rank limit likewise changed a recorded run.
  it('records a run under the length and squares it started with', async () => {
    const { wrapper, board, saveRun } = await trainer()
    await button(wrapper, 'Start').trigger('click')
    const asked = wrapper.find('.coord-target').text()
    window.kchess.viewPreferences = async () => ({
      'kchess:coordinates-length': 300,
      'kchess:coordinates-limited': true,
      'kchess:coordinates-files': ['a'],
      'kchess:coordinates-ranks': ['1'],
    })
    await loadViewPreferences()
    await flushPromises()
    board().vm.$emit('select', asked)
    await flushPromises()
    // The next square still comes from the whole board, not only a1.
    const targets = new Set<string>()
    for (let i = 0; i < 8; i++) {
      const target = wrapper.find('.coord-target').text()
      targets.add(target)
      board().vm.$emit('select', target)
      await flushPromises()
    }
    expect([...targets].some((target) => target !== 'a1')).toBe(true)
    await button(wrapper, 'End run').trigger('click')
    await flushPromises()
    expect(saveRun).toHaveBeenCalledWith(expect.objectContaining({ variant: 'find', score: 9 }))
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
    expect(wrapper.find('[role="status"]').text()).toBe(`Name the marked square: ${first[0]}`)
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

// The keyboard listener was added only after the saved scores loaded: after leaving the drill if
// they were slow, and never if they failed.
it('leaves no keyboard listener behind when its scores load after it closes', async () => {
  const summary = deferred<RunSummary>()
  desktop({ runSummary: () => summary.promise })
  await useKChessStore().init()
  const listeners = keydownListeners()
  const wrapper = mount(CoordinatesTrainer, { global: { stubs } })
  expect(listeners.count()).toBe(1)
  wrapper.unmount()
  summary.resolve({ kind: 'coordinates', best: {}, recent: [], total: 0 })
  await flushPromises()
  expect(listeners.count()).toBe(0)
  listeners.restore()
})

it('keeps keyboard controls when its saved scores cannot be read', async () => {
  desktop({ runSummary: () => Promise.reject(new Error('Disk unavailable')) })
  await useKChessStore().init()
  const listeners = keydownListeners()
  const wrapper = mount(CoordinatesTrainer, { global: { stubs } })
  await flushPromises()
  expect(listeners.count()).toBe(1)
  wrapper.unmount()
  expect(listeners.count()).toBe(0)
  listeners.restore()
})
