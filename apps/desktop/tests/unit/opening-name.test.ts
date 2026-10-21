import { expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { INITIAL_FEN } from '@kchess/rules/position'
import OpeningName from '../../app/components/OpeningName.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop } from './fixtures'

const setup = { variant: 'standard' as const, fen: INITIAL_FEN }

// The name's line appeared with the first named move, moving the banner and controls below it.
it('keeps its line before an opening is named, and none when names are off', async () => {
  desktop()
  const store = useKChessStore()
  await store.init()
  store.settings.showOpeningName = true
  const name = mount(OpeningName, { props: { setup, moves: [] } })
  expect(name.find('p.opening-name').exists()).toBe(true)
  expect(name.get('p.opening-name').text()).toBe('')
  await name.setProps({ moves: ['e2e4', 'c7c5'] })
  expect(name.get('p.opening-name').text()).toContain('Sicilian Defense')
  store.settings.showOpeningName = false
  await name.vm.$nextTick()
  expect(name.find('p.opening-name').exists()).toBe(false)
  name.unmount()
})
