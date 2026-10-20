import { expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import SettingsAccounts from '../../app/components/settings/SettingsAccounts.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { useUsageStore } from '../../app/stores/usage'
import { componentStubs, desktop } from './fixtures'

it('loads what each account keeps and downloaded when Accounts opens first', async () => {
  // Only the Data section asked for the numbers, so Accounts showed zero until it was opened.
  const usage = vi.fn(async () => ({
    accounts: { alice: { total: { requests: 3, bytesIn: 2048 }, byKind: {} } },
    storage: { alice: { games: 12, bytes: 1024, cacheBytes: 0 } },
  }))
  desktop({ usage: usage as never })
  await useKChessStore().init()
  const section = mount(SettingsAccounts, {
    global: { stubs: { ...componentStubs, AccountRow: true, ConfirmDialog: true } },
  })
  await flushPromises()
  expect(usage).toHaveBeenCalled()
  expect(useUsageStore().storageOf('Alice').games).toBe(12)
  section.unmount()
})
