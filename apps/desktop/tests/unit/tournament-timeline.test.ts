import { beforeEach, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import TournamentTimeline from '../../app/components/TournamentTimeline.vue'
import type { TournamentSummary } from '@kchess/contracts/types'

const MINUTE = 60_000
const start = Date.UTC(2026, 9, 6, 14)

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(start - 10 * MINUTE)
})

function arena(id: string, startsAt: number, minutes = 60): TournamentSummary {
  return {
    id,
    system: 'arena',
    name: id,
    status: 'created',
    variant: 'standard',
    variantName: 'Standard',
    rated: true,
    clock: { limit: 180, increment: 0 },
    minutes,
    startsAt,
    finishesAt: startsAt + minutes * MINUTE,
    nbPlayers: 0,
    playable: true,
    perf: 'blitz',
    freq: 'hourly',
  }
}

function render(arenas: TournamentSummary[]) {
  return mount(TournamentTimeline, {
    props: { arenas, isJoined: () => false },
    global: { stubs: { UIcon: true } },
  })
}

it('aligns events within the same minute with the time axis despite seconds and milliseconds', () => {
  const wrapper = render([arena('first', start), arena('second', start + 59_999)])
  const bars = wrapper.findAll<HTMLButtonElement>('.timeline-bar')
  expect(bars[0].element.style.left).toBe(bars[1].element.style.left)
  expect(bars[0].element.style.width).toBe(bars[1].element.style.width)
  const label = new Date(start).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
  const tick = wrapper.findAll<HTMLElement>('.timeline-tick').find((t) => t.text() === label)!
  expect(bars[0].element.style.left).toBe(tick.element.style.left)
  wrapper.unmount()
})

it('places back-to-back events in one lane when their minute boundaries touch', async () => {
  const wrapper = render([
    arena('next', start + 60 * MINUTE + 1000),
    arena('previous', start + 59_999),
  ])
  expect(wrapper.findAll('.timeline-row')).toHaveLength(1)
  const bars = wrapper.findAll<HTMLButtonElement>('.timeline-bar')
  expect(parseFloat(bars[0].element.style.left) + parseFloat(bars[0].element.style.width)).toBe(
    parseFloat(bars[1].element.style.left),
  )
  await bars[1].trigger('click')
  expect(wrapper.emitted('select')).toEqual([['next']])
  wrapper.unmount()
})

it('keeps genuine minute-level overlaps in separate lanes', () => {
  const wrapper = render([arena('previous', start), arena('next', start + 59 * MINUTE)])
  expect(wrapper.findAll('.timeline-row')).toHaveLength(2)
  wrapper.unmount()
})
