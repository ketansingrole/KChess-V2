import { describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent } from 'vue'
import Computer from '../../app/pages/computer.vue'
import Local from '../../app/pages/local.vue'
import CoordinatesTrainer from '../../app/components/CoordinatesTrainer.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { useLocalGameStore } from '../../app/stores/local'
import { desktop, deferred, componentStubs } from './fixtures'
import type { RunInput } from '@kchess/contracts/types'

const VoiceStub = defineComponent({
  name: 'VoiceInput',
  props: ['active', 'contextKey', 'grammar', 'enabled'],
  emits: ['update:enabled', 'result'],
  setup(_, { expose }) {
    expose({ prepare: async () => true })
  },
  template: '<div data-voice><slot /></div>',
})
const TabsStub = defineComponent({
  props: ['modelValue'],
  emits: ['update:modelValue'],
  template: '<div data-tabs />',
})
const stubs = {
  ...componentStubs,
  VoiceInput: VoiceStub,
  UTabs: TabsStub,
  USwitch: true,
  PageHeader: true,
  PlayBoard: { template: '<div><slot name="bottom-aside" /></div>' },
  MovePanel: { template: '<div><slot name="bottom" /></div>' },
  ConfirmDialog: true,
}

async function computer() {
  desktop({ bestMove: async () => 'e7e5' })
  vi.stubGlobal('useRoute', () => ({ path: '/computer' }))
  const store = useKChessStore()
  await store.init()
  const wrapper = mount(Computer, { global: { stubs } })
  await flushPromises()
  const voice = wrapper.findComponent(VoiceStub)
  voice.vm.$emit('update:enabled', true)
  await flushPromises()
  return { wrapper, voice, store }
}

describe('voice feature integration', () => {
  async function sharedBoard() {
    desktop()
    vi.stubGlobal('useRoute', () => ({ path: '/local', query: {} }))
    const store = useKChessStore()
    await store.init()
    const game = useLocalGameStore()
    game.start(undefined, {
      white: { minutes: 5, increment: 3 },
      black: { minutes: 5, increment: 3 },
    })
    const wrapper = mount(Local, { global: { stubs } })
    await flushPromises()
    const voice = wrapper.findComponent(VoiceStub)
    voice.vm.$emit('update:enabled', true)
    await flushPromises()
    return { wrapper, voice, store, game }
  }

  it('confirms shared-board moves for both players through the clock pipeline', async () => {
    const { wrapper, voice, game } = await sharedBoard()
    voice.vm.$emit('result', { text: 'e four', confidence: 0.95 })
    await flushPromises()
    expect(game.moves).toEqual([])
    expect(game.paused).toBe(true)
    expect(wrapper.text()).toContain('1 · e4')
    voice.vm.$emit('result', { text: 'confirm', confidence: 0.95 })
    await flushPromises()
    expect(game.moves).toEqual(['e2e4'])
    expect(game.paused).toBe(false)
    voice.vm.$emit('result', { text: 'e five', confidence: 0.95 })
    await flushPromises()
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '1 · e5')!
      .trigger('click')
    expect(game.moves).toEqual(['e2e4', 'e7e5'])
    expect(game.times!.black).toBeGreaterThan(300_000)
    wrapper.unmount()
  })

  it('clears shared-board choices after manual moves and same-position restarts', async () => {
    const { wrapper, voice, game } = await sharedBoard()
    voice.vm.$emit('result', { text: 'e four', confidence: 0.95 })
    await flushPromises()
    game.start()
    await flushPromises()
    expect(wrapper.text()).not.toContain('1 · e4')
    voice.vm.$emit('result', { text: 'confirm', confidence: 0.95 })
    expect(game.moves).toEqual([])
    voice.vm.$emit('result', { text: 'd four', confidence: 0.95 })
    await flushPromises()
    game.move('e2e4')
    await flushPromises()
    voice.vm.$emit('result', { text: 'confirm', confidence: 0.95 })
    expect(game.moves).toEqual(['e2e4'])
    expect(wrapper.text()).not.toContain('1 · d4')
    wrapper.unmount()
  })

  it('guards uncertain shared-board moves and stops voice during review or after game end', async () => {
    const { wrapper, voice, store, game } = await sharedBoard()
    store.settings.voiceConfirmMoves = false
    voice.vm.$emit('result', { text: 'e four', confidence: 0.5, unclear: true })
    await flushPromises()
    expect(game.moves).toEqual([])
    voice.vm.$emit('result', { text: 'cancel', confidence: 0.95 })
    await flushPromises()
    expect(wrapper.text()).not.toContain('1 · e4')
    voice.vm.$emit('result', { text: 'e four', confidence: 0.95 })
    await flushPromises()
    expect(game.moves).toEqual(['e2e4'])
    game.view(0)
    await flushPromises()
    expect(voice.props('active')).toBe(false)
    voice.vm.$emit('result', { text: 'e five', confidence: 0.95 })
    expect(game.moves).toEqual(['e2e4'])
    game.view(1)
    game.agreeDraw()
    await flushPromises()
    expect(voice.props('active')).toBe(false)
    voice.vm.$emit('result', { text: 'e five', confidence: 0.95 })
    expect(game.moves).toEqual(['e2e4'])
    wrapper.unmount()
  })

  it('requires confirmation and submits through the normal computer-game pipeline', async () => {
    const { wrapper, voice, store } = await computer()
    voice.vm.$emit('result', { text: 'echo two to echo four', confidence: 0.95 })
    await flushPromises()
    expect(store.localMoves).toEqual([])
    const choice = wrapper.findAll('button').find((button) => button.text() === '1 · e4')!
    expect(choice.exists()).toBe(true)
    await choice.trigger('click')
    await flushPromises()
    expect(store.localMoves).toEqual(['e2e4', 'e7e5'])
    expect(wrapper.text()).not.toContain('1 · e4')
    wrapper.unmount()
  })

  it('takes back and resigns by voice, asking before the game is given up', async () => {
    const { wrapper, voice, store } = await computer()
    store.settings.voiceConfirmMoves = false
    voice.vm.$emit('result', { text: 'e four', confidence: 0.95 })
    await flushPromises()
    expect(store.localMoves).toEqual(['e2e4', 'e7e5'])
    voice.vm.$emit('result', { text: 'take back', confidence: 0.95 })
    await flushPromises()
    expect(store.localMoves).toEqual([])
    voice.vm.$emit('result', { text: 'd four', confidence: 0.95 })
    await flushPromises()
    voice.vm.$emit('result', { text: 'resign', confidence: 0.95 })
    await flushPromises()
    expect(store.localResult).toBeNull()
    voice.vm.$emit('result', { text: 'confirm', confidence: 0.95 })
    await flushPromises()
    expect(store.localResult?.detail).toBe('You resigned')
    voice.vm.$emit('result', { text: 'new game', confidence: 0.95 })
    await flushPromises()
    expect(store.localMoves).toEqual([])
    expect(store.localResult).toBeNull()
    wrapper.unmount()
  })

  it('cancels pending moves when a new game has the same starting position', async () => {
    const { wrapper, voice, store } = await computer()
    voice.vm.$emit('result', { text: 'e four', confidence: 0.95 })
    await flushPromises()
    expect(wrapper.text()).toContain('1 · e4')
    store.newGame()
    await flushPromises()
    expect(wrapper.text()).not.toContain('1 · e4')
    voice.vm.$emit('result', { text: 'confirm', confidence: 0.95 })
    expect(store.localMoves).toEqual([])
    wrapper.unmount()
  })

  it('scores spoken squares independently of keyboard runs and waits for microphone readiness', async () => {
    const saveRun = vi.fn(async (run: RunInput) => ({
      isBest: true,
      summary: { kind: run.kind, best: {}, recent: [], total: 1 },
    }))
    desktop({ saveRun })
    await useKChessStore().init()
    vi.spyOn(Math, 'random').mockReturnValueOnce(0).mockReturnValueOnce(0.2).mockReturnValue(0.3)
    const ready = deferred<boolean>()
    const PreparingVoice = defineComponent({
      ...VoiceStub,
      setup(_, { expose }) {
        expose({ prepare: () => ready.promise })
      },
    })
    const wrapper = mount(CoordinatesTrainer, {
      global: { stubs: { ...stubs, VoiceInput: PreparingVoice } },
    })
    wrapper.findAllComponents(TabsStub)[0]!.vm.$emit('update:modelValue', 'voice')
    await flushPromises()
    const voice = wrapper.findComponent(PreparingVoice)
    voice.vm.$emit('update:enabled', true)
    await flushPromises()
    await wrapper
      .findAll('button')
      .find((button) => button.text() === 'Start')!
      .trigger('click')
    expect(wrapper.find('.run-hud').exists()).toBe(false)
    ready.resolve(true)
    await flushPromises()
    expect(wrapper.find('.run-hud').exists()).toBe(true)
    voice.vm.$emit('result', { text: 'alpha one', confidence: 0.95 })
    await flushPromises()
    expect(wrapper.find('.run-score strong').text()).toBe('1')
    await wrapper
      .findAll('button')
      .find((button) => button.text() === 'End run')!
      .trigger('click')
    expect(saveRun).toHaveBeenCalledWith(
      expect.objectContaining({ variant: 'name-voice', score: 1 }),
    )
    wrapper.unmount()
  })
})
