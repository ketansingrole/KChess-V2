import { defineStore } from 'pinia'
import { ref, watch } from 'vue'
import type { LichessStudy } from '../../src/shared/types'
import { useKChessStore } from './kchess'

const FRESH_FOR = 5 * 60_000
const RETRY_AFTER = 60_000
interface Entry {
  items: LichessStudy[] | null
  fetchedAt: number
  attemptedAt: number
  loading: boolean
  error: string
  needsReconnect: boolean
}

/** Session cache: navigation retains lists, while logging out removes private metadata. */
export const useLichessStudiesStore = defineStore('lichess-studies', () => {
  const app = useKChessStore()
  const entries = ref<Record<string, Entry>>({})
  const pending = new Map<string, Promise<void>>()
  const keyOf = (account: string) => account.toLowerCase()
  const connected = (account: string) =>
    app.connectedAccounts.some((a) => keyOf(a.username) === keyOf(account))

  watch(
    () => app.connectedAccounts.map((a) => keyOf(a.username)),
    (accounts) => {
      for (const key of Object.keys(entries.value)) {
        if (!accounts.includes(key)) {
          delete entries.value[key]
          pending.delete(key)
        }
      }
    },
    { flush: 'sync' },
  )

  function entry(account: string): Entry | undefined {
    return entries.value[keyOf(account)]
  }
  function refresh(account: string, force = false): Promise<void> {
    if (!account || !connected(account)) return Promise.resolve()
    const key = keyOf(account)
    if (pending.has(key)) return pending.get(key)!
    let cached = entries.value[key]
    if (!cached) {
      entries.value[key] = {
        items: null,
        fetchedAt: 0,
        attemptedAt: 0,
        loading: false,
        error: '',
        needsReconnect: false,
      }
      cached = entries.value[key]!
    }
    const now = Date.now()
    if (
      !force &&
      (cached.needsReconnect ||
        (cached.items !== null && now - cached.fetchedAt < FRESH_FOR) ||
        (cached.attemptedAt > 0 && now - cached.attemptedAt < RETRY_AFTER))
    )
      return Promise.resolve()
    cached.loading = true
    cached.error = ''
    cached.attemptedAt = now
    const request = (async () => {
      try {
        const result = await window.kchess.lichessStudies(account)
        if (!connected(account) || entries.value[key] !== cached) return
        if ('needsReconnect' in result) cached.needsReconnect = true
        else {
          cached.items = result
          cached.fetchedAt = Date.now()
          cached.needsReconnect = false
        }
      } catch (cause) {
        if (entries.value[key] === cached) {
          console.warn('[lichess-studies] loading studies failed:', cause)
          cached.error = cause instanceof Error ? cause.message : String(cause)
        }
      } finally {
        cached.loading = false
        if (entries.value[key] === cached) pending.delete(key)
      }
    })()
    pending.set(key, request)
    return request
  }
  return { entry, refresh }
})
