import { defineStore } from 'pinia'
import { useOnline } from '@vueuse/core'
import { computed, ref } from 'vue'
import type { FollowingReport, LichessUser, UserPresence } from '@kchess/core/contracts/types'
import { useKChessStore } from './kchess'
import { useUsageStore } from './usage'

const PRESENCE_CHUNK = 50

/** Live details of the players being followed: their Lichess profile and whether they are online. */
export const useFriendsStore = defineStore('friends', () => {
  const online = useOnline()
  const kchess = useKChessStore()
  const usage = useUsageStore()

  const profiles = ref<Record<string, LichessUser | null>>({})
  const status = ref<Record<string, UserPresence>>({})
  const statusAt = ref<number | undefined>()
  const loading = ref(false)

  const friends = computed(() => kchess.trackedAccounts)

  /** Saved profiles paint at once; fresh ones follow one at a time (Lichess asks for no parallel requests). */
  async function loadProfiles(): Promise<void> {
    loading.value = true
    // Profiles saved within the last hour (e.g. by an import) are not worth another request.
    const fresh = new Set<string>()
    try {
      for (const friend of friends.value) {
        const key = friend.username.toLowerCase()
        if (profiles.value[key]) continue
        const saved = await window.kchess.cachedProfile(friend.username).catch((error: unknown) => {
          console.warn('[friends] reading cached profile failed:', error)
          return null
        })
        if (saved?.profile) {
          profiles.value[key] = saved.profile
          if (saved.profileFetchedAt && Date.now() - saved.profileFetchedAt < 3_600_000)
            fresh.add(key)
        }
      }
      for (const friend of friends.value) {
        if (!online.value || fresh.has(friend.username.toLowerCase())) continue
        try {
          profiles.value[friend.username.toLowerCase()] = await window.kchess.profile(
            friend.username,
          )
        } catch (cause) {
          console.warn('[friends] loading profile failed:', cause)
          // Keep whatever was saved; the card simply shows fewer details.
        }
      }
    } finally {
      loading.value = false
      void usage.refresh()
    }
  }

  async function refreshStatus(): Promise<void> {
    if (!online.value) return
    const names = friends.value.map((friend) => friend.username)
    for (let start = 0; start < names.length; start += PRESENCE_CHUNK) {
      try {
        const report = await window.kchess.presence(names.slice(start, start + PRESENCE_CHUNK))
        Object.assign(status.value, report.users)
        statusAt.value = Date.now()
      } catch (cause) {
        console.warn('[friends] refreshing presence failed:', cause)
        // Statuses are a nicety; the next poll tries again.
      }
    }
  }

  const following = ref<FollowingReport | null>(null)
  const loadingFollowing = ref(false)
  const followingError = ref('')

  /** Ask Lichess whom the connected accounts follow. */
  async function loadFollowing(): Promise<void> {
    loadingFollowing.value = true
    followingError.value = ''
    try {
      following.value = await window.kchess.following()
    } catch (cause) {
      console.warn('[friends] loading following failed:', cause)
      following.value = null
      followingError.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      loadingFollowing.value = false
      void usage.refresh()
    }
  }

  /** Drop what is remembered about someone who stopped being a friend. */
  function forget(username: string): void {
    const key = username.toLowerCase()
    delete profiles.value[key]
    delete status.value[key]
  }

  const onlineCount = computed(
    () =>
      friends.value.filter((friend) => status.value[friend.username.toLowerCase()]?.online).length,
  )

  return {
    profiles,
    status,
    statusAt,
    loading,
    friends,
    onlineCount,
    following,
    loadingFollowing,
    followingError,
    loadFollowing,
    loadProfiles,
    refreshStatus,
    forget,
  }
})
