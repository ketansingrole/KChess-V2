import { expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent, watchEffect } from 'vue'
import LichessStudies from '../../app/components/LichessStudies.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { componentStubs, desktop } from './fixtures'

// Reka UI reserves an empty select item value for clearing the selection.
const SelectStub = defineComponent({
  props: ['items', 'modelValue'],
  emits: ['update:modelValue'],
  setup(props) {
    watchEffect(() => {
      for (const item of props.items) {
        if (item.value === '') throw new Error('Select items cannot have empty values')
      }
    })
  },
  template: `<select :value="modelValue" @change="$emit('update:modelValue', $event.target.value)">
    <option v-for="item in items" :key="item.value" :value="item.value">{{ item.label }}</option>
  </select>`,
})

it('renders the Lichess studies tab and exports to either a new or an existing study', async () => {
  const exportToLichessStudy = vi.fn(async () => ({ id: 'Study001' }))
  desktop({
    exportToLichessStudy,
    lichessStudies: async () => [{ id: 'Study001', name: 'My repertoire', updatedAt: 0 }],
  })
  await useKChessStore().init()
  const wrapper = mount(LichessStudies, {
    global: { stubs: { ...componentStubs, ConfirmDialog: true, USelect: SelectStub } },
  })
  const target = wrapper.get('select[aria-label="Lichess study to add to"]')
  expect(target.findAll('option').map((option) => option.text())).toEqual(['A new private study'])
  await wrapper.get('form').trigger('submit')
  await flushPromises()
  expect(exportToLichessStudy).toHaveBeenLastCalledWith(
    expect.any(String),
    '',
    'KChess analysis',
    expect.any(String),
  )
  await target.setValue('Study001')
  await wrapper.get('form').trigger('submit')
  await flushPromises()
  expect(exportToLichessStudy).toHaveBeenLastCalledWith(
    expect.any(String),
    'Study001',
    'KChess analysis',
    expect.any(String),
  )
  wrapper.unmount()
})

it('loads automatically, reuses the list on return, and refreshes stale data on focus', async () => {
  let now = 1_000_000
  vi.spyOn(Date, 'now').mockImplementation(() => now)
  const lichessStudies = vi.fn(async () => [{ id: 'Study001', name: 'Cached study', updatedAt: 0 }])
  desktop({ lichessStudies })
  await useKChessStore().init()
  const render = () =>
    mount(LichessStudies, {
      global: { stubs: { ...componentStubs, ConfirmDialog: true, USelect: SelectStub } },
    })
  let wrapper = render()
  await flushPromises()
  expect(lichessStudies).toHaveBeenCalledTimes(1)
  expect(wrapper.text()).toContain('Cached study')
  wrapper.unmount()
  wrapper = render()
  expect(wrapper.text()).toContain('Cached study')
  await flushPromises()
  expect(lichessStudies).toHaveBeenCalledTimes(1)
  now += 5 * 60_000
  window.dispatchEvent(new Event('blur'))
  await flushPromises()
  window.dispatchEvent(new Event('focus'))
  await flushPromises()
  expect(lichessStudies).toHaveBeenCalledTimes(2)
  wrapper.unmount()
})

it('keeps account caches separate and retains a cached list when refresh fails', async () => {
  const lichessStudies = vi.fn(async (account: string) => [
    { id: 'Study001', name: `${account} study`, updatedAt: 0 },
  ])
  desktop({ lichessStudies })
  await useKChessStore().init()
  const wrapper = mount(LichessStudies, {
    global: { stubs: { ...componentStubs, ConfirmDialog: true, USelect: SelectStub } },
  })
  await flushPromises()
  const account = wrapper.get('select[aria-label="Account"]')
  await account.setValue('Bob')
  expect(wrapper.text()).not.toContain('Alice study')
  await flushPromises()
  expect(wrapper.text()).toContain('Bob study')
  await account.setValue('Alice')
  await flushPromises()
  expect(wrapper.text()).toContain('Alice study')
  expect(lichessStudies).toHaveBeenCalledTimes(2)
  lichessStudies.mockRejectedValueOnce(new Error('Connection failed'))
  await wrapper
    .findAll('button')
    .find((button) => button.text() === 'Refresh')!
    .trigger('click')
  await flushPromises()
  expect(wrapper.text()).toContain('Alice study')
  expect(wrapper.get('[role="status"]').text()).toBe('Connection failed')
  wrapper.unmount()
})

it('deduplicates loading and discards private cached data and late replies after logout', async () => {
  const { deferred } = await import('./fixtures')
  const { useLichessStudiesStore } = await import('../../app/stores/lichessStudies')
  const response = deferred<[{ id: string; name: string; updatedAt: number }]>()
  const lichessStudies = vi.fn(() => response.promise)
  desktop({ lichessStudies })
  const app = useKChessStore()
  await app.init()
  const cache = useLichessStudiesStore()
  const first = cache.refresh('Alice')
  const second = cache.refresh('alice')
  expect(lichessStudies).toHaveBeenCalledTimes(1)
  app.data!.accounts = []
  response.resolve([{ id: 'Study001', name: 'Private study', updatedAt: 0 }])
  await Promise.all([first, second])
  expect(cache.entry('Alice')).toBeUndefined()
})

it('downloads a cloud study as one offline study with all chapters', async () => {
  const { useStudyStore } = await import('../../app/stores/studies')
  desktop({
    lichessStudies: async () => [{ id: 'Study001', name: 'Cloud repertoire', updatedAt: 0 }],
    lichessStudyChapters: async () => [
      { name: 'King pawn', pgn: '1. e4 e5 *' },
      { name: 'Queen pawn', pgn: '1. d4 d5 *' },
    ],
  })
  await useKChessStore().init()
  const wrapper = mount(LichessStudies, {
    global: {
      stubs: { ...componentStubs, ConfirmDialog: true, USelect: SelectStub },
    },
  })
  await flushPromises()
  await wrapper
    .findAll('button')
    .find((b) => b.text() === 'Make available offline')!
    .trigger('click')
  await flushPromises()
  const library = useStudyStore()
  expect(library.items).toHaveLength(1)
  expect(library.items[0]!.name).toBe('Cloud repertoire')
  expect(library.items[0]!.chapters.map((c) => c.name)).toEqual(['King pawn', 'Queen pawn'])
  expect(wrapper.text()).toContain('Download latest')
  await wrapper
    .findAll('button')
    .find((b) => b.text() === 'Download latest')!
    .trigger('click')
  await flushPromises()
  expect(library.items).toHaveLength(1)
  wrapper.unmount()
})
