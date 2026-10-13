import { afterEach, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import { DEFAULT_SETTINGS } from '@kchess/contracts/defaultSettings'
import type { Settings } from '@kchess/contracts/types'
import { useSettingsPersistence } from '../../app/stores/kchess/settingsPersistence'
import { deferred, desktop } from './fixtures'

const scopes: ReturnType<typeof effectScope>[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
  vi.useRealTimers()
})

function persistence() {
  vi.useFakeTimers()
  const settings = ref<Settings | null>({ ...DEFAULT_SETTINGS })
  const data = ref({ settings: { ...DEFAULT_SETTINGS }, accounts: [], gameCount: 0 })
  const fail = vi.fn()
  const scope = effectScope()
  scopes.push(scope)
  const owner = scope.run(() =>
    useSettingsPersistence({ settings, data, fail, refreshEngine: async () => {} }),
  )!
  return { settings, data, fail, ...owner }
}

it('preserves edits made during a failed save and persists them on the next save', async () => {
  const pending = deferred<Settings>()
  const saveSettings = vi.fn(async (value: Settings) => ({ ...value }))
  saveSettings.mockImplementationOnce(() => pending.promise)
  desktop({ saveSettings })
  const owner = persistence()
  owner.settings.value!.appearance = 'dark'
  const first = owner.save()
  await nextTick()
  expect(saveSettings).toHaveBeenCalledOnce()
  owner.settings.value!.appearance = 'light'
  pending.reject(new Error('Disk unavailable'))
  await first
  expect(owner.settings.value!.appearance).toBe('light')
  expect(owner.fail).toHaveBeenCalledOnce()
  await owner.save()
  expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ appearance: 'light' }))
  expect(owner.data.value.settings.appearance).toBe('light')
})

it('restores saved settings when the failed request still matches the form', async () => {
  desktop({
    saveSettings: async () => {
      throw new Error('Disk unavailable')
    },
  })
  const owner = persistence()
  const saved = owner.data.value.settings.appearance
  owner.settings.value!.appearance = saved === 'dark' ? 'light' : 'dark'
  await owner.save()
  expect(owner.settings.value!.appearance).toBe(saved)
  expect(owner.settingsDirty.value).toBe(false)
})
