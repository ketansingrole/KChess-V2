import { expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { nextTick } from 'vue'
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
  await expect(flushViewPreferences()).rejects.toThrow('kchess:migrated-pref')
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

it('retries a refused preference before quitting and reports one still refused', async () => {
  let full = true
  const stored: Record<string, unknown> = {}
  const write = vi.fn(async (key: string, value: unknown) => {
    if (full) throw new Error('disk full')
    stored[key] = value
  })
  desktop({ viewPreferences: async () => ({}), setViewPreference: write })
  await loadViewPreferences()
  useViewPreference('kchess:refused-pref', 'old').value = 'new'
  await nextTick()
  await flushPromises()
  expect(write).toHaveBeenCalledTimes(1)
  // Quitting tries it again and does not report success while it is still unsaved.
  await expect(flushViewPreferences()).rejects.toThrow('kchess:refused-pref')
  expect(write).toHaveBeenCalledTimes(2)
  // Once the disk recovers, the next quit saves it.
  full = false
  await flushViewPreferences()
  expect(stored['kchess:refused-pref']).toBe('new')
  await flushViewPreferences()
  expect(write).toHaveBeenCalledTimes(3)
})

it('forgets a refused preference once a later change to it is saved', async () => {
  let full = true
  const write = vi.fn(async () => {
    if (full) throw new Error('disk full')
  })
  desktop({ viewPreferences: async () => ({}), setViewPreference: write })
  await loadViewPreferences()
  const pref = useViewPreference('kchess:superseded-pref', 'old')
  pref.value = 'refused'
  await nextTick()
  await flushPromises()
  full = false
  pref.value = 'saved'
  await nextTick()
  await flushViewPreferences()
  expect(write).toHaveBeenLastCalledWith('kchess:superseded-pref', 'saved')
  expect(write).toHaveBeenCalledTimes(2)
})

it('takes another app’s change to a shown preference without saving it back', async () => {
  let saved: Record<string, unknown> = { 'kchess:studies-tab': 'device' }
  const setViewPreference = vi.fn(async (key: string, value: unknown) => void (saved[key] = value))
  desktop({ viewPreferences: async () => ({ ...saved }), setViewPreference })
  await loadViewPreferences()
  const tab = useViewPreference('kchess:studies-tab', 'device')
  saved = { 'kchess:studies-tab': 'lichess', 'kchess:new-page': 'grid' }
  await loadViewPreferences()
  await nextTick()
  await flushPromises()
  expect(tab.value).toBe('lichess')
  // A page first opened after the change shows it too.
  expect(useViewPreference('kchess:new-page', 'list').value).toBe('grid')
  expect(setViewPreference).not.toHaveBeenCalled()
})

it('keeps this app’s newer change when a read from before it arrives', async () => {
  let reply: (value: Record<string, unknown>) => void = () => {}
  desktop({
    viewPreferences: () => new Promise((resolve) => (reply = resolve)),
    setViewPreference: async () => {},
  })
  const first = loadViewPreferences()
  reply({ 'kchess:studies-tab': 'device' })
  await first
  const tab = useViewPreference('kchess:studies-tab', 'device')
  const stale = loadViewPreferences()
  tab.value = 'lichess'
  await nextTick()
  reply({ 'kchess:studies-tab': 'device' })
  await stale
  await flushPromises()
  expect(tab.value).toBe('lichess')
})
