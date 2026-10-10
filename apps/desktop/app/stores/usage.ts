import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import type { AccountStorage, AccountUsage, UsageReport } from '@kchess/contracts/types'

const EMPTY_USAGE: AccountUsage = { total: { requests: 0, bytesIn: 0 }, byKind: {} }
const EMPTY_STORAGE: AccountStorage = { games: 0, bytes: 0, cacheBytes: 0 }

/** What KChess has downloaded from Lichess for each account, and what it keeps on this computer. */
export const useUsageStore = defineStore('usage', () => {
  const report = ref<UsageReport | null>(null)
  const loading = ref(false)

  async function refresh(): Promise<UsageReport | null> {
    loading.value = true
    try {
      report.value = await window.kchess.usage()
    } catch (cause) {
      console.warn('[usage] refreshing usage failed:', cause)
      // Accounting is informational; keep showing the last numbers.
    } finally {
      loading.value = false
    }
    return report.value
  }

  async function reset(): Promise<void> {
    await window.kchess.resetUsage()
    await refresh()
  }

  const usageOf = (username: string): AccountUsage =>
    report.value?.accounts[username.toLowerCase()] ?? EMPTY_USAGE
  const storageOf = (username: string): AccountStorage =>
    report.value?.storage[username.toLowerCase()] ?? EMPTY_STORAGE

  /** Bytes downloaded across every account and purpose. */
  const totalDownloaded = computed(() =>
    Object.values(report.value?.accounts ?? {}).reduce((sum, a) => sum + a.total.bytesIn, 0),
  )
  const totalRequests = computed(() =>
    Object.values(report.value?.accounts ?? {}).reduce((sum, a) => sum + a.total.requests, 0),
  )
  const totalStored = computed(() =>
    Object.values(report.value?.storage ?? {}).reduce((sum, s) => sum + s.bytes + s.cacheBytes, 0),
  )

  return {
    report,
    loading,
    refresh,
    reset,
    usageOf,
    storageOf,
    totalDownloaded,
    totalRequests,
    totalStored,
  }
})
