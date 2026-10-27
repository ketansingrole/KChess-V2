import { expect, it } from 'vitest'
import { desktop } from './fixtures'
import {
  flushViewPreferences,
  loadViewPreferences,
  resetViewPreferences,
  useViewPreference,
} from '../../app/composables/useViewPreference'

it('keeps an earlier release’s preference until the core has saved it', async () => {
  desktop({
    viewPreferences: async () => ({}),
    setViewPreference: async () => {
      throw new Error('Disk unavailable')
    },
  })
  localStorage.setItem('kchess:migrated-pref', 'previous value')
  await loadViewPreferences()
  expect(useViewPreference('kchess:migrated-pref', 'default').value).toBe('previous value')
  await flushViewPreferences()
  expect(localStorage.getItem('kchess:migrated-pref')).toBe('previous value')

  // The next start moves it again, and this time the core keeps it.
  resetViewPreferences()
  const stored: Record<string, unknown> = {}
  desktop({
    viewPreferences: async () => ({}),
    setViewPreference: async (key, value) => void (stored[key] = value),
  })
  await loadViewPreferences()
  expect(useViewPreference('kchess:migrated-pref', 'default').value).toBe('previous value')
  await flushViewPreferences()
  expect(stored['kchess:migrated-pref']).toBe('previous value')
  expect(localStorage.getItem('kchess:migrated-pref')).toBeNull()
})
