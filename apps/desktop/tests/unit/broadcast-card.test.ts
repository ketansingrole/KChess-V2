import { mount } from '@vue/test-utils'
import { expect, it } from 'vitest'
import BroadcastCard from '../../app/components/BroadcastCard.vue'
import type { BroadcastSummary } from '@kchess/core/contracts/types'

const item: BroadcastSummary = {
  tourId: 'tour1234',
  tourName: 'Test Open',
  roundName: 'Round 4',
  ongoing: false,
  startsAt: Date.now() + 30 * 60_000,
  section: 'upcoming',
  image: 'https://image.lichess1.org/test.webp',
}
it('shows event artwork, scheduled timing, live status and opens the selected event', async () => {
  const wrapper = mount(BroadcastCard, { props: { item }, global: { stubs: { UIcon: true } } })
  expect(wrapper.get('img').attributes('src')).toBe(item.image)
  expect(wrapper.text()).toContain('in 30 min')
  expect(wrapper.text()).not.toContain('LIVE')
  await wrapper.get('button').trigger('click')
  expect(wrapper.emitted('select')).toHaveLength(1)
  await wrapper.setProps({ item: { ...item, ongoing: true, section: 'active' } })
  expect(wrapper.text()).toContain('LIVE')
  await wrapper.get('img').trigger('error')
  expect(wrapper.find('img').exists()).toBe(false)
  await wrapper.setProps({ disabled: true })
  expect(wrapper.get('button').attributes('disabled')).toBeDefined()
  wrapper.unmount()
})
