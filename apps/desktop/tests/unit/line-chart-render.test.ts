import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick, ref } from 'vue'
import type { LichessRatingHistory } from '@kchess/contracts/types'
import LineChart from '../../app/components/LineChart.vue'
import RatingChart from '../../app/components/RatingChart.vue'
import RatingHistoryChart from '../../app/components/RatingHistoryChart.vue'
import ScoreSparkline from '../../app/components/ScoreSparkline.vue'

// happy-dom has no layout: give every chart a fixed width.
vi.mock('@vueuse/core', async (original) => ({
  ...(await original<typeof import('@vueuse/core')>()),
  useElementSize: () => ({ width: ref(600), height: ref(240) }),
}))

const global = {
  components: { LineChart },
  stubs: {
    UFieldGroup: { template: '<div><slot /></div>' },
    UButton: { template: '<button><slot /></button>' },
  },
}
async function hover(wrapper: ReturnType<typeof mount>, clientX: number): Promise<void> {
  await wrapper.find('.line-chart').trigger('pointermove', { clientX })
  await nextTick()
}

describe('charts on the shared line chart', () => {
  it('draws a rating line with axes and shows the hovered day as text', async () => {
    const points = [
      { date: Date.UTC(2026, 0, 1), rating: 1500 },
      { date: Date.UTC(2026, 0, 11), rating: 1540 },
      { date: Date.UTC(2026, 0, 21), rating: 1520 },
    ]
    const chart = mount(RatingChart, { props: { points, label: 'Blitz' }, global })
    await nextTick()
    expect(chart.attributes('aria-label')).toBe('Blitz rating history from 1500 to 1540')
    const paths = chart.findAll('path')
    expect(paths).toHaveLength(2) // area and line
    expect(paths[1]!.attributes('d')).toMatch(/^M[\d.]+,[\d.]+C/)
    expect(chart.findAll('.line-chart-axis text').length).toBeGreaterThan(3)
    expect(chart.find('.line-chart-tooltip').exists()).toBe(false)

    await hover(chart, 590) // the right edge: the last point
    const tooltip = chart.find('.line-chart-tooltip')
    expect(tooltip.text()).toContain('1520')
    expect(tooltip.attributes('style')).toContain('right:') // flipped to the left of the line
    await chart.find('.line-chart').trigger('pointerleave')
    expect(chart.find('.line-chart-tooltip').exists()).toBe(false)
  })

  it('keeps speed names as text in the history tooltip and leaves gaps before a first game', async () => {
    const start = Date.UTC(2026, 0, 1)
    const history: LichessRatingHistory = [
      {
        name: 'blitz',
        points: [
          [2026, 0, 1, 1500],
          [2026, 0, 3, 1510],
          [2026, 0, 5, 1490],
        ],
      },
      {
        name: 'rapid',
        points: [
          [2026, 0, 3, 1600],
          [2026, 0, 5, 1610],
        ],
      },
    ]
    const chart = mount(RatingHistoryChart, { props: { history }, global })
    await nextTick()
    const lines = chart.findAll('.line-chart path')
    expect(lines).toHaveLength(2)
    expect(chart.findAll('.line-chart-crosshair').length).toBe(0)

    await hover(chart, 0) // the first day: only Blitz has a rating
    const tooltip = chart.find('.rh-tip')
    expect(tooltip.findAll('.rh-tip-line').map((line) => line.text())).toEqual(['Blitz1500'])
    expect(chart.findAll('.line-chart-crosshair circle')).toHaveLength(1)
    expect(tooltip.text()).toContain(new Date(start).getUTCFullYear().toString())

    await hover(chart, 600)
    expect(chart.findAll('.rh-tip-line').map((line) => line.text())).toEqual([
      'Blitz1490',
      'Rapid1610',
    ])
  })

  it('draws the sparkline without axes or tooltip', async () => {
    const chart = mount(ScoreSparkline, {
      props: { scores: [3, 5, 4], label: 'Recent scores' },
      global,
    })
    await nextTick()
    expect(chart.findAll('.line-chart-axis')).toHaveLength(0)
    expect(chart.findAll('path')).toHaveLength(2)
    await hover(chart, 300)
    expect(chart.find('.line-chart-tooltip').exists()).toBe(false)
  })
})
