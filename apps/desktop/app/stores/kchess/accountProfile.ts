import { ref, watch, type Ref } from 'vue'
import type { LichessUser, LichessRatingHistory } from '@kchess/core/contracts/types'

/** Cached profile refresh with account/request ownership; stale responses cannot replace newer data. */
export function useAccountProfile(selectedAccount: Ref<string>, fail: (cause: unknown) => void) {
  const profile = ref<LichessUser | null>(null)
  const ratingHistories = ref<LichessRatingHistory>([])
  function applyRatingHistory(history: LichessRatingHistory): void {
    ratingHistories.value = history
  }
  let epoch = 0
  async function loadProfile(): Promise<void> {
    const request = ++epoch
    const account = selectedAccount.value
    if (!account) {
      profile.value = null
      ratingHistories.value = []
      return
    }
    try {
      // Paint what was saved last time straight away, then refresh from Lichess.
      const saved = await window.kchess.cachedProfile(account)
      if (request !== epoch || account !== selectedAccount.value) return
      profile.value = saved.profile
      if (saved.ratingHistory) applyRatingHistory(saved.ratingHistory)
      else ratingHistories.value = []
      const [p, history] = await Promise.all([
        window.kchess.profile(account),
        window.kchess.ratingHistory(account),
      ])
      if (request !== epoch || account !== selectedAccount.value) return
      profile.value = p
      applyRatingHistory(history)
    } catch (cause) {
      if (request === epoch) {
        console.warn('[account-profile] loading profile failed:', cause)
        fail(cause)
      }
    }
  }
  watch(selectedAccount, () => {
    void loadProfile()
  })
  return { profile, ratingHistories, loadProfile }
}
