import { expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, nextTick, ref, type Ref } from 'vue'
import { useZenSurface } from '../../app/composables/useZenSurface'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop } from './fixtures'

const Surface = (shown: Ref<boolean> | boolean) =>
  defineComponent({
    setup() {
      useZenSurface(shown)
      return () => null
    },
  })

const pressZ = () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'z' }))

/*
 * Zen mode used to apply to whole pages, including a lobby or the chess clock without a zen toggle,
 * so a separate floating "Leave zen mode" button was the only way out. It now applies only beside a
 * board that shows its toggle.
 */
it('applies zen mode only while a board with its toggle is shown', async () => {
  desktop()
  const store = useKChessStore()
  await store.init()
  store.settings!.zenMode = true
  expect(store.zenActive).toBe(false)
  const board = ref(true)
  const page = mount(Surface(board))
  expect(store.zenActive).toBe(true)
  board.value = false // e.g. Over the board switching to the chess clock
  await nextTick()
  expect(store.zenActive).toBe(false)
  page.unmount()
})

it('keeps zen mode when the next page registers before the last one leaves', async () => {
  desktop()
  const store = useKChessStore()
  await store.init()
  store.settings!.zenMode = true
  const previous = mount(Surface(true))
  const next = mount(Surface(true))
  previous.unmount()
  expect(store.zenActive).toBe(true)
  next.unmount()
  expect(store.zenActive).toBe(false)
})

it('toggles zen mode with Z only beside a board', async () => {
  desktop()
  const store = useKChessStore()
  await store.init()
  store.settings!.zenMode = false
  pressZ()
  expect(store.settings!.zenMode).toBe(false)
  const page = mount(Surface(true))
  pressZ()
  expect(store.settings!.zenMode).toBe(true)
  expect(store.zenActive).toBe(true)
  pressZ()
  expect(store.settings!.zenMode).toBe(false)
  page.unmount()
})
