import { expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import type { PerfStats, PerfType } from '@kchess/contracts/types'
import Players from '../../app/pages/players.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { componentStubs, deferred, desktop } from './fixtures'

const Select = defineComponent({
  props: ['modelValue', 'items'],
  emits: ['update:modelValue'],
  template: `<select :value="modelValue" @change="$emit('update:modelValue', $event.target.value)">
    <option v-for="item in items" :key="item.value" :value="item.value">{{ item.label }}</option>
  </select>`,
})
function stats(perf: PerfType, rating: number): PerfStats {
  return {
    perf,
    rating,
    count: {
      all: 20,
      rated: 20,
      win: 10,
      loss: 10,
      draw: 0,
      tour: 0,
      berserk: 0,
      opAvg: 1500,
      seconds: 600,
      disconnects: 0,
    },
    bestWins: [],
    worstLosses: [],
    winStreak: { current: 0, best: 0 },
    lossStreak: { current: 0, best: 0 },
  }
}
async function render() {
  vi.stubGlobal('useRoute', () => ({ path: '/players', query: { name: 'Carol' } }))
  await useKChessStore().init()
  const wrapper = mount(Players, {
    global: {
      stubs: {
        ...componentStubs,
        USelect: Select,
        UInput: true,
        UEmpty: true,
        UBadge: true,
        UAlert: true,
        UModal: true,
        UTextarea: true,
        PageHeader: true,
        PublicOnlineNotice: true,
        RatingHistoryChart: true,
        ResultBar: true,
        PlayerLink: true,
        GameRow: true,
      },
    },
  })
  await flushPromises()
  return wrapper
}
function bridge(playerPerf: (name: string, perf: PerfType) => Promise<PerfStats>) {
  desktop({
    profile: async () => ({
      username: 'Carol',
      id: 'carol',
      url: 'https://lichess.org/@/Carol',
      perfs: {
        blitz: { games: 20, rating: 1500, rd: 50, prog: 0 },
        rapid: { games: 10, rating: 1800, rd: 50, prog: 0 },
      },
    }),
    recentGames: async () => [],
    crosstable: async () => ({ users: {}, nbGames: 0 }),
    playerPerf,
  })
}

it('keeps the selected speed when an older stats response arrives last', async () => {
  const old = deferred<PerfStats>()
  const next = deferred<PerfStats>()
  const playerPerf = vi.fn((_name: string, speed: PerfType) =>
    speed === 'blitz' ? old.promise : next.promise,
  )
  bridge(playerPerf)
  const wrapper = await render()
  expect(playerPerf).toHaveBeenCalledWith('Carol', 'blitz')
  await wrapper.get('select[aria-label="Speed"]').setValue('rapid')
  next.resolve(stats('rapid', 1800))
  await flushPromises()
  expect(wrapper.get('.record-rating').text()).toBe('1800')
  old.resolve(stats('blitz', 1500))
  await flushPromises()
  expect(wrapper.get('.record-rating').text()).toBe('1800')
  wrapper.unmount()
})

it('clears the previous speed while loading and ignores its late error', async () => {
  const old = deferred<PerfStats>()
  const rapid = deferred<PerfStats>()
  const playerPerf = vi.fn(async (_name: string, speed: PerfType) =>
    speed === 'rapid' ? rapid.promise : stats('blitz', 1500),
  )
  bridge(playerPerf)
  const wrapper = await render()
  expect(wrapper.get('.record-rating').text()).toBe('1500')
  playerPerf.mockImplementationOnce(() => old.promise)
  // Rapid -> blitz -> rapid creates overlapping requests within the same profile lookup.
  await wrapper.get('select[aria-label="Speed"]').setValue('rapid')
  expect(wrapper.find('.record-rating').exists()).toBe(false)
  await wrapper.get('select[aria-label="Speed"]').setValue('blitz')
  await wrapper.get('select[aria-label="Speed"]').setValue('rapid')
  rapid.resolve(stats('rapid', 1800))
  await flushPromises()
  old.reject(new Error('Old request failed'))
  await flushPromises()
  expect(wrapper.get('.record-rating').text()).toBe('1800')
  expect(wrapper.text()).not.toContain('Old request failed')
  wrapper.unmount()
})
