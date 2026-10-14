import { expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import SettingsAppearance from '../../app/components/settings/SettingsAppearance.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { boardThemes } from '../../app/utils/boards'
import { pieceSets } from '../../app/utils/pieces'
import { componentStubs, desktop } from './fixtures'

// The board themes were read through `storeToRefs`, which drops plain values: the grid was empty.
it('offers every board theme and piece set, and picks a board theme', async () => {
  desktop()
  const app = useKChessStore()
  await app.init()
  const wrapper = mount(SettingsAppearance, {
    global: { stubs: { ...componentStubs, UTabs: true, USelect: true, USwitch: true } },
  })
  const group = (label: string) =>
    wrapper.get(`[role="radiogroup"][aria-labelledby="${label}"]`).findAll('[role="radio"]')
  expect(group('board-theme-label')).toHaveLength(boardThemes.length)
  expect(group('piece-set-label')).toHaveLength(pieceSets.length)
  const other = boardThemes.find((theme) => theme.id !== app.settings!.boardTheme)!
  const choice = group('board-theme-label').find((radio) => radio.text() === other.name)!
  await choice.trigger('click')
  expect(app.settings!.boardTheme).toBe(other.id)
  expect(choice.attributes('aria-checked')).toBe('true')
  wrapper.unmount()
})
